//! M0 resize gate (ARCHITECTURE.md 5.2 and 16), tier 3. Launches `examples/hello` with its window
//! background painted magenta (`ND_DEBUG_EXPOSE=1`), captures every frame the window server
//! presents for that window through ScreenCaptureKit, and drags the window's edges with HID
//! CGEvents, so AppKit runs a real live resize. Every captured frame must be free of magenta
//! (content covered the window to its edge) and show the close button at the configured point.
//! Then full screen is entered and left with the menu's key equivalent, and a last drag checks
//! the lights again. A control run with `ND_DEBUG_DEFER_LIVE_LAYOUT=1` must fail the magenta
//! check, which proves the detector sees a late layout.
//!
//! Needs Screen Recording and Accessibility (post events) for the terminal running it, and a
//! built `hello` (`cargo build -p hello`, or `ND_HELLO_BIN`). Prints `ND_M0_RESIZE_GATE_OK`.
//! `ND_GATE_OUT=<dir>` keeps a mid-drag and a final frame as BMP.

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    harness::main_real("real_gate_m0::resize_drag", || gate::run());
    #[cfg(not(target_os = "macos"))]
    harness::main_real("real_gate_m0::resize_drag", || {});
}

#[cfg(target_os = "macos")]
mod gate {
    use std::io::{BufRead, BufReader};
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use block2::RcBlock;
    use dispatch2::DispatchQueue;
    use nd_backend_appkit::objc2::rc::Retained;
    use nd_backend_appkit::objc2::runtime::ProtocolObject;
    use nd_backend_appkit::objc2::{AnyThread, DefinedClass, define_class, msg_send};
    use nd_backend_appkit::objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSScreen};
    use nd_backend_appkit::objc2_foundation::{
        MainThreadMarker, NSDate, NSError, NSObject, NSObjectProtocol, NSRunLoop,
    };
    use objc2_core_foundation::CGPoint;
    use objc2_core_graphics::{CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton};
    use objc2_core_media::{CMSampleBuffer, kCMTimeZero};
    use objc2_core_video::{
        CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetWidth,
        CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
    };
    use objc2_screen_capture_kit::{
        SCContentFilter, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamOutput, SCStreamOutputType,
    };

    /// Where `examples/hello` pins its close button, in points from the window's top-left.
    const LIGHTS: (f64, f64) = (20., 20.);
    const BUTTON: f64 = 14.;
    const BGRA: u32 = u32::from_be_bytes(*b"BGRA");

    struct SendBox<T>(T);
    // SAFETY: only moves retained ScreenCaptureKit objects from a completion handler to the
    // waiting thread, which uses them alone afterwards.
    unsafe impl<T> Send for SendBox<T> {}

    #[derive(Clone, Copy, Debug, Default)]
    struct Frame {
        width: usize,
        height: usize,
        magenta: u32,
        lights_ok: bool,
    }

    #[derive(Default)]
    struct State {
        phase: Option<&'static str>,
        frames: Vec<(&'static str, Frame)>,
        scale: f64,
        save: Option<PathBuf>,
    }

    pub struct SinkIvars {
        state: Arc<Mutex<State>>,
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
        #[unsafe(super(NSObject))]
        #[ivars = SinkIvars]
        struct FrameSink;

        unsafe impl NSObjectProtocol for FrameSink {}

        unsafe impl SCStreamOutput for FrameSink {
            #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
            unsafe fn did_output(&self, _stream: &SCStream, sample: &CMSampleBuffer, kind: SCStreamOutputType) {
                if kind == SCStreamOutputType::Screen {
                    self.frame(sample);
                }
            }
        }
    );

    impl FrameSink {
        fn new(state: Arc<Mutex<State>>) -> Retained<Self> {
            let this = Self::alloc().set_ivars(SinkIvars { state });
            unsafe { msg_send![super(this), init] }
        }

        fn frame(&self, sample: &CMSampleBuffer) {
            // Idle and blank frames carry no image.
            let Some(image) = (unsafe { sample.image_buffer() }) else { return };
            let mut state = self.ivars().state.lock().unwrap();
            let Some(phase) = state.phase else { return };
            unsafe { CVPixelBufferLockBaseAddress(&image, CVPixelBufferLockFlags::ReadOnly) };
            let width = CVPixelBufferGetWidth(&image);
            let height = CVPixelBufferGetHeight(&image);
            let stride = CVPixelBufferGetBytesPerRow(&image);
            let base = CVPixelBufferGetBaseAddress(&image).cast::<u8>();
            let pixels = unsafe { std::slice::from_raw_parts(base, stride * height) };
            let frame = analyze(pixels, width, height, stride, state.scale);
            if frame.is_some()
                && let Some(path) = state.save.take()
            {
                write_bmp(&path, pixels, width, height, stride);
            }
            unsafe { CVPixelBufferUnlockBaseAddress(&image, CVPixelBufferLockFlags::ReadOnly) };
            if let Some(frame) = frame {
                state.frames.push((phase, frame));
            }
        }
    }

    fn px(pixels: &[u8], stride: usize, x: usize, y: usize) -> [u8; 4] {
        let i = y * stride + x * 4;
        [pixels[i + 2], pixels[i + 1], pixels[i], pixels[i + 3]]
    }

    fn is_magenta([r, g, b, a]: [u8; 4]) -> bool {
        a > 200 && r > 200 && g < 70 && b > 200
    }

    /// The window sits at the top-left of the buffer (independent-window capture, no scaling);
    /// its extent is where the pixels turn opaque.
    fn analyze(p: &[u8], width: usize, height: usize, stride: usize, scale: f64) -> Option<Frame> {
        let mid_y = (height.min(200)).min(height - 1);
        let right = (0..width).rev().find(|&x| px(p, stride, x, mid_y)[3] > 0)? + 1;
        let mid_x = right / 2;
        let bottom = (0..height).rev().find(|&y| px(p, stride, mid_x, y)[3] > 0)? + 1;
        // Rounded corners are transparent, not magenta; stay clear of them anyway.
        let corner = (40. * scale) as usize;
        let band = (48. * scale) as usize;
        let mut magenta = 0;
        for y in corner..bottom.saturating_sub(corner) {
            for x in right.saturating_sub(band)..right {
                magenta += u32::from(is_magenta(px(p, stride, x, y)));
            }
        }
        for y in bottom.saturating_sub(band)..bottom {
            for x in corner..right.saturating_sub(corner) {
                magenta += u32::from(is_magenta(px(p, stride, x, y)));
            }
        }
        for y in (0..bottom).step_by(6) {
            for x in (0..right).step_by(6) {
                magenta += u32::from(is_magenta(px(p, stride, x, y)));
            }
        }
        let cx = ((LIGHTS.0 + BUTTON / 2.) * scale) as usize;
        let cy = ((LIGHTS.1 + BUTTON / 2.) * scale) as usize;
        let [r, g, b, _] = px(p, stride, cx, cy);
        let lights_ok = r > 180 && g < 130 && b < 130;
        Some(Frame { width: right, height: bottom, magenta, lights_ok })
    }

    fn write_bmp(path: &PathBuf, p: &[u8], width: usize, height: usize, stride: usize) {
        let mut out = Vec::with_capacity(54 + width * height * 4);
        let size = (54 + width * height * 4) as u32;
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(width as i32).to_le_bytes());
        out.extend_from_slice(&(-(height as i32)).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&[0; 24]);
        for y in 0..height {
            out.extend_from_slice(&p[y * stride..y * stride + width * 4]);
        }
        let _ = std::fs::write(path, out);
    }

    struct Hello {
        child: Child,
        lines: Arc<Mutex<Vec<String>>>,
    }

    impl Hello {
        fn launch(defer_layout: bool) -> Self {
            let bin = std::env::var_os("ND_HELLO_BIN").map(PathBuf::from).unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/hello")
            });
            assert!(bin.exists(), "build hello first (cargo build -p hello) or set ND_HELLO_BIN; looked at {bin:?}");
            let mut child = Command::new(bin)
                .env("ND_DIAG_WINDOW", "1")
                .env("ND_DEBUG_EXPOSE", "1")
                .env("ND_DEBUG_DEFER_LIVE_LAYOUT", if defer_layout { "1" } else { "0" })
                .stderr(Stdio::piped())
                .spawn()
                .expect("hello starts");
            let lines = Arc::new(Mutex::new(Vec::new()));
            let stderr = child.stderr.take().unwrap();
            let sink = lines.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    sink.lock().unwrap().push(line);
                }
            });
            Self { child, lines }
        }

        fn count(&self, needle: &str) -> usize {
            self.lines.lock().unwrap().iter().filter(|l| l.contains(needle)).count()
        }

        fn wait_for(&self, needle: &str, at_least: usize, timeout: Duration) -> bool {
            let end = Instant::now() + timeout;
            while Instant::now() < end {
                if self.count(needle) >= at_least {
                    return true;
                }
                spin(Duration::from_millis(20));
            }
            false
        }

        /// The latest `nd: window frame x y w h number=n` line.
        fn frame(&self) -> Option<(f64, f64, f64, f64, u32)> {
            let lines = self.lines.lock().unwrap();
            let line = lines.iter().rev().find(|l| l.starts_with("nd: window frame "))?;
            let mut it = line["nd: window frame ".len()..].split_whitespace();
            let mut num = || it.next()?.parse::<f64>().ok();
            let (x, y, w, h) = (num()?, num()?, num()?, num()?);
            let number = line.rsplit("number=").next()?.parse().ok()?;
            Some((x, y, w, h, number))
        }

        fn last_lights(&self) -> Option<String> {
            let lines = self.lines.lock().unwrap();
            lines.iter().rev().find(|l| l.starts_with("nd: traffic lights")).cloned()
        }
    }

    impl Drop for Hello {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    /// Waits while running the main run loop, where ScreenCaptureKit delivers its completions.
    fn spin(d: Duration) {
        let until = NSDate::dateWithTimeIntervalSinceNow(d.as_secs_f64());
        NSRunLoop::currentRunLoop().runUntilDate(&until);
    }

    fn post_mouse(kind: CGEventType, x: f64, y: f64) {
        let event = CGEvent::new_mouse_event(None, kind, CGPoint { x, y }, CGMouseButton::Left);
        CGEvent::post(CGEventTapLocation::HIDEventTap, event.as_deref());
    }

    fn post_key(code: u16, flags: CGEventFlags) {
        for down in [true, false] {
            let event = CGEvent::new_keyboard_event(None, code, down);
            CGEvent::set_flags(event.as_deref(), flags);
            CGEvent::post(CGEventTapLocation::HIDEventTap, event.as_deref());
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    /// Presses at `from`, drags by `(dx, dy)` in `steps` HID moves at about 80 Hz, releases.
    fn drag(from: (f64, f64), dx: f64, dy: f64, steps: u32) {
        post_mouse(CGEventType::MouseMoved, from.0, from.1);
        std::thread::sleep(Duration::from_millis(150));
        eprintln!("debug: drag from {from:?}, cursor now {:?}", nd_backend_appkit::objc2_app_kit::NSEvent::mouseLocation());
        post_mouse(CGEventType::LeftMouseDown, from.0, from.1);
        std::thread::sleep(Duration::from_millis(60));
        for i in 1..=steps {
            let t = f64::from(i) / f64::from(steps);
            post_mouse(CGEventType::LeftMouseDragged, from.0 + dx * t, from.1 + dy * t);
            std::thread::sleep(Duration::from_millis(12));
        }
        std::thread::sleep(Duration::from_millis(60));
        post_mouse(CGEventType::LeftMouseUp, from.0 + dx, from.1 + dy);
        std::thread::sleep(Duration::from_millis(300));
    }

    fn start_capture(window_number: u32, state: &Arc<Mutex<State>>, max: (f64, f64)) -> Retained<SCStream> {
        let found: Arc<Mutex<Option<SendBox<Retained<SCShareableContent>>>>> = Arc::new(Mutex::new(None));
        let done = found.clone();
        let block = RcBlock::new(move |content: *mut SCShareableContent, _error: *mut NSError| {
            let content = unsafe { Retained::retain(content) };
            *done.lock().unwrap() = content.map(SendBox);
        });
        unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&block) };
        let end = Instant::now() + Duration::from_secs(10);
        let content = loop {
            if let Some(content) = found.lock().unwrap().take() {
                break content.0;
            }
            assert!(Instant::now() < end, "ScreenCaptureKit gave no shareable content (Screen Recording permission?)");
            spin(Duration::from_millis(20));
        };
        let windows = unsafe { content.windows() };
        let window = windows
            .iter()
            .find(|w| unsafe { w.windowID() } == window_number)
            .expect("hello's window is shareable");
        let filter = unsafe { SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &window) };
        let config = unsafe { SCStreamConfiguration::new() };
        unsafe {
            config.setWidth(max.0 as usize);
            config.setHeight(max.1 as usize);
            // Zero: every frame the display presents, at its native rate.
            config.setMinimumFrameInterval(kCMTimeZero);
            config.setPixelFormat(BGRA);
            config.setScalesToFit(false);
            config.setShowsCursor(false);
            config.setIgnoreShadowsSingleWindow(true);
            config.setQueueDepth(8);
        }
        let stream = unsafe { SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), &filter, &config, None) };
        let sink = FrameSink::new(state.clone());
        let queue = DispatchQueue::new("nd.gate.frames", None);
        unsafe {
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(
                    ProtocolObject::from_ref(&*sink),
                    SCStreamOutputType::Screen,
                    Some(&queue),
                )
                .expect("stream output");
        }
        // The stream holds its outputs weakly.
        std::mem::forget(sink);
        let started = Arc::new(Mutex::new(None::<bool>));
        let flag = started.clone();
        let block = RcBlock::new(move |error: *mut NSError| {
            *flag.lock().unwrap() = Some(error.is_null());
        });
        unsafe { stream.startCaptureWithCompletionHandler(Some(&block)) };
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(ok) = *started.lock().unwrap() {
                assert!(ok, "SCStream failed to start");
                break;
            }
            assert!(
                Instant::now() < end,
                "SCStream did not start ({} frames arrived)",
                state.lock().unwrap().frames.len()
            );
            spin(Duration::from_millis(20));
        }
        stream
    }

    struct Report {
        failures: Vec<String>,
    }

    impl Report {
        fn check(&mut self, ok: bool, what: String) {
            println!("{} {what}", if ok { "ok  " } else { "FAIL" });
            if !ok {
                self.failures.push(what);
            }
        }
    }

    fn set_phase(state: &Arc<Mutex<State>>, phase: Option<&'static str>) {
        state.lock().unwrap().phase = phase;
    }

    fn phase_frames(state: &Arc<Mutex<State>>, phase: &str) -> Vec<Frame> {
        state.lock().unwrap().frames.iter().filter(|(p, _)| *p == phase).map(|(_, f)| *f).collect()
    }

    /// Drags the right edge then the bottom edge. Returns the frames of both drags.
    fn edge_drags(hello: &Hello, state: &Arc<Mutex<State>>, phase: &'static str, grow: f64) -> Vec<Frame> {
        let (x, y, w, h, _) = hello.frame().expect("hello reports its frame");
        let before = hello.count("live resize start");
        set_phase(state, Some(phase));
        drag((x + w + 1., y + h / 2.), grow, 0., 50);
        if hello.count("live resize start") == before {
            // The resize zone sits inside the edge on some releases.
            drag((x + w - 2., y + h / 2.), grow, 0., 50);
        }
        drag((x + (w + grow) / 2., y + h + 1.), 0., grow * 0.6, 50);
        hello.wait_for("live resize end", before + 2, Duration::from_secs(2));
        std::thread::sleep(Duration::from_millis(200));
        set_phase(state, None);
        phase_frames(state, phase)
    }

    /// A launched-in-background app is not active (activation is cooperative); a real click on
    /// an empty part of its window makes it so, like a user would.
    fn activate(hello: &Hello) {
        let (x, y, w, h, _) = hello.frame().expect("hello reports its frame");
        let at = (x + w / 2., y + h - 30.);
        post_mouse(CGEventType::MouseMoved, at.0, at.1);
        std::thread::sleep(Duration::from_millis(100));
        post_mouse(CGEventType::LeftMouseDown, at.0, at.1);
        std::thread::sleep(Duration::from_millis(50));
        post_mouse(CGEventType::LeftMouseUp, at.0, at.1);
        spin(Duration::from_millis(400));
    }

    fn summarize(frames: &[Frame]) -> (usize, usize, usize, usize) {
        let mut widths: Vec<(usize, usize)> = frames.iter().map(|f| (f.width, f.height)).collect();
        widths.sort_unstable();
        widths.dedup();
        let magenta = frames.iter().filter(|f| f.magenta > 0).count();
        let lights = frames.iter().filter(|f| f.lights_ok).count();
        (frames.len(), widths.len(), magenta, lights)
    }

    /// The control: with layout deferred during live resize, magenta has to show.
    fn control_run(scale: f64, max: (f64, f64), r: &mut Report) {
        let hello = Hello::launch(true);
        if !hello.wait_for("window key", 1, Duration::from_secs(10)) {
            r.check(false, "control: hello's window became key".into());
            return;
        }
        let state = Arc::new(Mutex::new(State { scale, ..Default::default() }));
        let (.., number) = hello.frame().unwrap();
        let stream = start_capture(number, &state, max);
        let frames = edge_drags(&hello, &state, "control", 160.);
        unsafe { stream.stopCaptureWithCompletionHandler(None) };
        let (n, _, magenta, _) = summarize(&frames);
        r.check(
            magenta > 0,
            format!("control: with layout deferred, {magenta} of {n} captured frames show the window background"),
        );
    }

    pub fn run() {
        let mtm = MainThreadMarker::new().expect("main thread");
        // A window server connection for ScreenCaptureKit; this process shows no UI.
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
        let screen = NSScreen::mainScreen(mtm).expect("a display");
        let scale = screen.backingScaleFactor();
        let size = screen.frame().size;
        let max = (size.width * scale, size.height * scale);
        let out = std::env::var_os("ND_GATE_OUT").map(PathBuf::from);
        if let Some(out) = &out {
            let _ = std::fs::create_dir_all(out);
        }
        let mut r = Report { failures: Vec::new() };

        let hello = Hello::launch(false);
        r.check(hello.wait_for("window key", 1, Duration::from_secs(10)), "hello's window is up and key".into());
        let (.., number) = hello.frame().expect("hello reports its frame");
        let state = Arc::new(Mutex::new(State { scale, ..Default::default() }));
        let stream = start_capture(number, &state, max);

        if let Some(out) = &out {
            state.lock().unwrap().save = Some(out.join("before.bmp"));
            set_phase(&state, Some("idle"));
            post_mouse(CGEventType::MouseMoved, 10., 10.);
            std::thread::sleep(Duration::from_millis(300));
        }

        activate(&hello);
        let frames = edge_drags(&hello, &state, "drag", 240.);
        let starts = hello.count("live resize start");
        r.check(starts >= 2, format!("HID edge drags put the window in live resize ({starts} live resizes)"));
        let (n, widths, magenta, lights) = summarize(&frames);
        r.check(n >= 30 && widths >= 10, format!("SCStream captured {n} presented frames at {widths} distinct window sizes"));
        r.check(magenta == 0, format!("content reaches the window edge in every frame ({magenta} of {n} frames show background)"));
        r.check(lights == n, format!("close button at {LIGHTS:?} in {lights} of {n} frames during resize"));

        post_key(0x03, CGEventFlags::MaskCommand | CGEventFlags::MaskControl);
        let entered = hello.wait_for("full screen enter", 1, Duration::from_secs(5));
        r.check(entered, "ctrl-cmd-f enters full screen".into());
        std::thread::sleep(Duration::from_millis(800));
        post_key(0x03, CGEventFlags::MaskCommand | CGEventFlags::MaskControl);
        let exited = hello.wait_for("full screen exit", 1, Duration::from_secs(5));
        r.check(exited, "ctrl-cmd-f leaves full screen".into());
        std::thread::sleep(Duration::from_millis(800));
        let lights_line = hello.last_lights().unwrap_or_default();
        r.check(
            lights_line.ends_with(&format!("close={},{}", LIGHTS.0, LIGHTS.1)),
            format!("after full screen the host reports `{lights_line}`"),
        );

        if let Some(out) = &out {
            state.lock().unwrap().save = Some(out.join("after-full-screen.bmp"));
        }
        let frames = edge_drags(&hello, &state, "after", -120.);
        let (n, _, magenta, lights) = summarize(&frames);
        r.check(
            n >= 10 && magenta == 0 && lights == n,
            format!("after full screen: {n} frames, {magenta} with background, close button placed in {lights}"),
        );
        unsafe { stream.stopCaptureWithCompletionHandler(None) };
        let ticks = hello.lines.lock().unwrap().iter().filter(|l| l.contains("frame clock")).count();
        drop(hello);

        if std::env::var_os("ND_GATE_SKIP_CONTROL").is_none() {
            control_run(scale, max, &mut r);
        }
        println!("frame clock reports from hello: {ticks}");

        if r.failures.is_empty() {
            println!("ND_M0_RESIZE_GATE_OK");
            std::process::exit(0);
        }
        println!("ND_M0_RESIZE_GATE_FAILED {} check(s)", r.failures.len());
        std::process::exit(1);
    }
}
