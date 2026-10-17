//! Real input and capture on a [`Hyprland`] instance, as a Wayland client of it: a virtual pointer
//! (`zwlr_virtual_pointer_v1`) and keyboard (`zwp_virtual_keyboard_v1`) that the compositor treats
//! like hardware, so it runs its own resize, move and focus logic; and `ext-image-copy-capture-v1`
//! of the output, which hands back every frame the compositor presents.

use std::fs::File;
use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_image_capture_source_v1, ext_output_image_capture_source_manager_v1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1, ext_image_copy_capture_manager_v1, ext_image_copy_capture_session_v1,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1, zwp_virtual_keyboard_v1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{zwlr_virtual_pointer_manager_v1, zwlr_virtual_pointer_v1};

use crate::hyprland::Hyprland;

/// Linux evdev codes (`linux/input-event-codes.h`) for the keys tests press.
pub mod keys {
    pub const ESC: u32 = 1;
    pub const TAB: u32 = 15;
    pub const ENTER: u32 = 28;
    pub const LEFTCTRL: u32 = 29;
    pub const A: u32 = 30;
    pub const S: u32 = 31;
    pub const LEFTSHIFT: u32 = 42;
    pub const LEFTALT: u32 = 56;
    pub const SPACE: u32 = 57;
    pub const LEFTMETA: u32 = 125;
}

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    Shift,
    Control,
    Alt,
    Super,
}

fn connect(hyprland: &Hyprland) -> Result<Connection, String> {
    connect_socket(&hyprland.wayland_socket())
}

fn connect_socket(socket: &std::path::Path) -> Result<Connection, String> {
    let stream = UnixStream::connect(socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    Connection::from_socket(stream).map_err(|e| format!("wayland connect: {e}"))
}

fn bind<I, S>(globals: &GlobalList, qh: &QueueHandle<S>, version: std::ops::RangeInclusive<u32>) -> Result<I, String>
where
    I: wayland_client::Proxy + 'static,
    S: Dispatch<I, ()> + 'static,
{
    globals.bind::<I, S, ()>(qh, version, ()).map_err(|e| format!("{}: {e}", I::interface().name))
}

/// Ignores registry changes after the initial list.
macro_rules! ignore_registry {
    ($state:ty) => {
        impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for $state {
            fn event(
                _: &mut Self,
                _: &wl_registry::WlRegistry,
                _: wl_registry::Event,
                _: &GlobalListContents,
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    };
}

struct InputState;
ignore_registry!(InputState);
delegate_noop!(InputState: ignore wl_seat::WlSeat);
delegate_noop!(InputState: ignore wl_output::WlOutput);
delegate_noop!(InputState: zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1);
delegate_noop!(InputState: zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1);
delegate_noop!(InputState: zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1);
delegate_noop!(InputState: zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1);

/// A virtual pointer and keyboard on the instance's seat. Coordinates are logical pixels of the
/// output, which sits at the layout origin.
pub struct VirtualInput {
    conn: Connection,
    queue: EventQueue<InputState>,
    pointer: zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
    keyboard: zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
    extent: (u32, u32),
    position: (f64, f64),
    mod_masks: [u32; 4],
    started: Instant,
}

impl VirtualInput {
    pub fn connect(hyprland: &Hyprland) -> Result<Self, String> {
        Self::connect_to(connect(hyprland)?, (hyprland.options.width, hyprland.options.height))
    }

    /// On the compositor behind `socket`, whose first output is `extent` logical pixels: the
    /// user's own session, for measurements that need its configuration and display.
    pub fn connect_socket(socket: &std::path::Path, extent: (u32, u32)) -> Result<Self, String> {
        Self::connect_to(connect_socket(socket)?, extent)
    }

    fn connect_to(conn: Connection, extent: (u32, u32)) -> Result<Self, String> {
        let (globals, queue) = registry_queue_init::<InputState>(&conn).map_err(|e| e.to_string())?;
        let qh = queue.handle();
        let seat: wl_seat::WlSeat = bind(&globals, &qh, 1..=1)?;
        let output: wl_output::WlOutput = bind(&globals, &qh, 1..=1)?;
        let pointers: zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1 = bind(&globals, &qh, 2..=2)?;
        let keyboards: zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1 = bind(&globals, &qh, 1..=1)?;
        let pointer = pointers.create_virtual_pointer_with_output(Some(&seat), Some(&output), &qh, ());
        let keyboard = keyboards.create_virtual_keyboard(&seat, &qh, ());

        let context = xkbcommon::xkb::Context::new(xkbcommon::xkb::CONTEXT_NO_FLAGS);
        let keymap = xkbcommon::xkb::Keymap::new_from_names(
            &context,
            "",
            "",
            "us",
            "",
            None,
            xkbcommon::xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .ok_or("xkbcommon could not compile the us keymap")?;
        let text = keymap.get_as_string(xkbcommon::xkb::KEYMAP_FORMAT_TEXT_V1);
        let mask = |name: &str| {
            let index = keymap.mod_get_index(name);
            if index == xkbcommon::xkb::MOD_INVALID { 0 } else { 1 << index }
        };
        let mod_masks = [
            mask(xkbcommon::xkb::MOD_NAME_SHIFT),
            mask(xkbcommon::xkb::MOD_NAME_CTRL),
            mask(xkbcommon::xkb::MOD_NAME_ALT),
            mask(xkbcommon::xkb::MOD_NAME_LOGO),
        ];
        let fd = rustix::fs::memfd_create("nd-keymap", rustix::fs::MemfdFlags::CLOEXEC).map_err(|e| e.to_string())?;
        let mut file = File::from(fd);
        file.write_all(text.as_bytes()).and_then(|()| file.write_all(&[0])).map_err(|e| e.to_string())?;
        keyboard.keymap(xkbcommon::xkb::KEYMAP_FORMAT_TEXT_V1, file.as_fd(), text.len() as u32 + 1);

        let mut this =
            Self { conn, queue, pointer, keyboard, extent, position: (0., 0.), mod_masks, started: Instant::now() };
        this.sync()?;
        Ok(this)
    }

    fn time(&self) -> u32 {
        self.started.elapsed().as_millis() as u32
    }

    /// Sends everything queued and waits until the compositor has processed it.
    pub fn sync(&mut self) -> Result<(), String> {
        self.queue.roundtrip(&mut InputState).map(|_| ()).map_err(|e| format!("wayland: {e}"))
    }

    fn flush(&self) {
        let _ = self.conn.flush();
    }

    pub fn position(&self) -> (f64, f64) {
        self.position
    }

    pub fn move_to(&mut self, x: f64, y: f64) {
        let (w, h) = self.extent;
        let clamp = |v: f64, max: u32| v.round().clamp(0., f64::from(max - 1)) as u32;
        self.pointer.motion_absolute(self.time(), clamp(x, w), clamp(y, h), w, h);
        self.pointer.frame();
        self.flush();
        self.position = (x, y);
    }

    pub fn button(&mut self, pressed: bool) {
        self.press(BTN_LEFT, pressed);
    }

    fn press(&mut self, button: u32, pressed: bool) {
        let state = if pressed { wl_pointer::ButtonState::Pressed } else { wl_pointer::ButtonState::Released };
        self.pointer.button(self.time(), button, state);
        self.pointer.frame();
        self.flush();
    }

    /// Presses and releases the secondary button at `(x, y)`.
    pub fn right_click(&mut self, x: f64, y: f64) -> Result<(), String> {
        self.move_to(x, y);
        self.sync()?;
        self.press(BTN_RIGHT, true);
        self.press(BTN_RIGHT, false);
        self.sync()
    }

    /// Turns the wheel `notches` steps at `(x, y)`: positive scrolls down, as a wheel turned
    /// towards the user does.
    pub fn scroll(&mut self, x: f64, y: f64, notches: i32) -> Result<(), String> {
        self.move_to(x, y);
        self.sync()?;
        for _ in 0..notches.unsigned_abs() {
            let step = notches.signum();
            let time = self.time();
            self.pointer.axis_source(wl_pointer::AxisSource::Wheel);
            self.pointer.axis_discrete(time, wl_pointer::Axis::VerticalScroll, f64::from(15 * step), step);
            self.pointer.frame();
            self.flush();
            std::thread::sleep(Duration::from_millis(16));
        }
        self.sync()
    }

    pub fn click(&mut self, x: f64, y: f64) -> Result<(), String> {
        self.move_to(x, y);
        self.sync()?;
        self.button(true);
        self.button(false);
        self.sync()
    }

    /// Presses at `from`, holds 100 ms the way a hand does (an app that answers the press with
    /// `xdg_toplevel.move` gets its request in before the pointer leaves), moves by `(dx, dy)` in
    /// `steps` motions `step` apart, and releases.
    pub fn drag(&mut self, from: (f64, f64), dx: f64, dy: f64, steps: u32, step: Duration) -> Result<(), String> {
        self.move_to(from.0, from.1);
        self.sync()?;
        // Long enough for the app to take pointer focus before the press.
        std::thread::sleep(step.max(Duration::from_millis(50)));
        self.button(true);
        self.sync()?;
        std::thread::sleep(Duration::from_millis(100));
        for i in 1..=steps {
            let t = f64::from(i) / f64::from(steps);
            self.move_to(from.0 + dx * t, from.1 + dy * t);
            std::thread::sleep(step);
        }
        self.sync()?;
        self.button(false);
        self.sync()
    }

    fn set_modifiers(&mut self, mods: &[Modifier]) {
        let depressed = mods.iter().fold(0, |acc, m| {
            acc | match m {
                Modifier::Shift => self.mod_masks[0],
                Modifier::Control => self.mod_masks[1],
                Modifier::Alt => self.mod_masks[2],
                Modifier::Super => self.mod_masks[3],
            }
        });
        self.keyboard.modifiers(depressed, 0, 0, 0);
    }

    /// Presses and releases `key` (an evdev code, see [`keys`]) with `mods` held.
    pub fn key(&mut self, key: u32, mods: &[Modifier]) -> Result<(), String> {
        const PRESSED: u32 = 1;
        const RELEASED: u32 = 0;
        if !mods.is_empty() {
            self.set_modifiers(mods);
        }
        self.keyboard.key(self.time(), key, PRESSED);
        self.keyboard.key(self.time(), key, RELEASED);
        if !mods.is_empty() {
            self.set_modifiers(&[]);
        }
        self.sync()
    }
}

/// One frame the compositor presented, as XRGB8888 or ARGB8888 (B, G, R, X bytes in memory), or
/// XBGR8888 or ABGR8888 (R, G, B, X) where the compositor offers only those, as labwc does.
pub struct CapturedFrame {
    /// The bytes are R, G, B, X rather than B, G, R, X.
    pub red_first: bool,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub data: Vec<u8>,
    /// From `presentation_time`, on the compositor's presentation clock.
    pub presented: Option<Duration>,
    /// 0 for the first frame of a capture.
    pub sequence: u64,
}

impl CapturedFrame {
    pub fn rgb(&self, x: u32, y: u32) -> [u8; 3] {
        let i = (y * self.stride + x * 4) as usize;
        if self.red_first {
            [self.data[i], self.data[i + 1], self.data[i + 2]]
        } else {
            [self.data[i + 2], self.data[i + 1], self.data[i]]
        }
    }

    /// The frame as a 24-bit BMP, for keeping evidence of a failed check.
    pub fn write_bmp(&self, path: &std::path::Path) -> std::io::Result<()> {
        let row = (self.width * 3).div_ceil(4) * 4;
        let size = 54 + row * self.height;
        let mut out = Vec::with_capacity(size as usize);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(self.width as i32).to_le_bytes());
        out.extend_from_slice(&(-(self.height as i32)).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&24u16.to_le_bytes());
        out.extend_from_slice(&[0; 24]);
        for y in 0..self.height {
            for x in 0..self.width {
                let [r, g, b] = self.rgb(x, y);
                out.extend_from_slice(&[b, g, r]);
            }
            out.resize(out.len() + (row - self.width * 3) as usize, 0);
        }
        std::fs::write(path, out)
    }
}

#[derive(Default)]
struct CaptureState {
    size: Option<(u32, u32)>,
    formats: Vec<wl_shm::Format>,
    constraints: bool,
    stopped: bool,
    presented: Option<Duration>,
    result: Option<Result<(), String>>,
}

ignore_registry!(CaptureState);
delegate_noop!(CaptureState: ignore wl_output::WlOutput);
delegate_noop!(CaptureState: ignore wl_shm::WlShm);
delegate_noop!(CaptureState: wl_shm_pool::WlShmPool);
delegate_noop!(CaptureState: ignore wl_buffer::WlBuffer);
delegate_noop!(CaptureState: ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(CaptureState: ext_image_capture_source_v1::ExtImageCaptureSourceV1);
delegate_noop!(CaptureState: ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1);

impl Dispatch<ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1, ()> for CaptureState {
    fn event(
        state: &mut Self,
        _: &ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_session_v1::Event;
        match event {
            Event::BufferSize { width, height } => state.size = Some((width, height)),
            Event::ShmFormat { format: WEnum::Value(format) } => state.formats.push(format),
            Event::Done => state.constraints = true,
            Event::Stopped => state.stopped = true,
            _ => {}
        }
    }
}

impl Dispatch<ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1, ()> for CaptureState {
    fn event(
        state: &mut Self,
        _: &ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_frame_v1::Event;
        match event {
            Event::PresentationTime { tv_sec_hi, tv_sec_lo, tv_nsec } => {
                let secs = (u64::from(tv_sec_hi) << 32) | u64::from(tv_sec_lo);
                state.presented = Some(Duration::new(secs, tv_nsec));
            }
            Event::Ready => state.result = Some(Ok(())),
            Event::Failed { reason } => state.result = Some(Err(format!("capture failed: {reason:?}"))),
            _ => {}
        }
    }
}

/// Dispatches until `done` holds, `stop` is set, or `timeout` passes. Polls so a stop request
/// is seen while the compositor has nothing new to present.
fn dispatch_until(
    conn: &Connection,
    queue: &mut EventQueue<CaptureState>,
    state: &mut CaptureState,
    stop: &AtomicBool,
    timeout: Duration,
    done: impl Fn(&CaptureState) -> bool,
) -> Result<bool, String> {
    let end = Instant::now() + timeout;
    loop {
        queue.dispatch_pending(state).map_err(|e| format!("wayland: {e}"))?;
        if done(state) {
            return Ok(true);
        }
        if stop.load(Ordering::Relaxed) || Instant::now() > end {
            return Ok(false);
        }
        queue.flush().map_err(|e| format!("wayland: {e}"))?;
        if let Some(guard) = queue.prepare_read() {
            let fd = guard.connection_fd();
            let mut fds = [rustix::event::PollFd::from_borrowed_fd(fd, rustix::event::PollFlags::IN)];
            let wait = rustix::event::Timespec { tv_sec: 0, tv_nsec: 20_000_000 };
            let ready = rustix::event::poll(&mut fds, Some(&wait)).map_err(|e| e.to_string())?;
            if ready > 0 {
                guard.read().map_err(|e| format!("wayland: {e}"))?;
            }
        }
        let _ = conn.flush();
    }
}

/// Captures every frame the instance presents on its output until [`Capture::stop`], handing
/// each to a callback on the capture thread (frames are not kept, they are 4 MB each).
pub struct Capture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<u64, String>>>,
}

impl Capture {
    pub fn start(hyprland: &Hyprland, on_frame: impl FnMut(&CapturedFrame) + Send + 'static) -> Result<Self, String> {
        Self::start_socket(&hyprland.wayland_socket(), on_frame)
    }

    /// As [`Capture::start`], on the compositor behind `socket` (any with
    /// `ext-image-copy-capture-v1`, such as labwc).
    pub fn start_socket(
        socket: &std::path::Path,
        mut on_frame: impl FnMut(&CapturedFrame) + Send + 'static,
    ) -> Result<Self, String> {
        let conn = connect_socket(socket)?;
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let flag = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut run = || -> Result<u64, String> {
                let (globals, mut queue) = registry_queue_init::<CaptureState>(&conn).map_err(|e| e.to_string())?;
                let qh = queue.handle();
                let output: wl_output::WlOutput = bind(&globals, &qh, 1..=1)?;
                let shm: wl_shm::WlShm = bind(&globals, &qh, 1..=1)?;
                let sources: ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1 =
                    bind(&globals, &qh, 1..=1)?;
                let manager: ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1 =
                    bind(&globals, &qh, 1..=1)?;
                let source = sources.create_source(&output, &qh, ());
                let session =
                    manager.create_session(&source, ext_image_copy_capture_manager_v1::Options::empty(), &qh, ());
                let mut state = CaptureState::default();
                let never = AtomicBool::new(false);
                if !dispatch_until(&conn, &mut queue, &mut state, &never, Duration::from_secs(5), |s| s.constraints)? {
                    return Err("no buffer constraints from the capture session".into());
                }
                let (width, height) = state.size.ok_or("capture session sent no buffer size")?;
                let format = [
                    wl_shm::Format::Xrgb8888,
                    wl_shm::Format::Argb8888,
                    wl_shm::Format::Xbgr8888,
                    wl_shm::Format::Abgr8888,
                ]
                .into_iter()
                .find(|f| state.formats.contains(f))
                .ok_or_else(|| format!("no 32-bit shm format offered: {:?}", state.formats))?;
                let stride = width * 4;
                let len = stride * height;
                let fd = rustix::fs::memfd_create("nd-capture", rustix::fs::MemfdFlags::CLOEXEC)
                    .map_err(|e| e.to_string())?;
                let file = File::from(fd);
                file.set_len(u64::from(len) * 2).map_err(|e| e.to_string())?;
                // Two buffers: the next capture is already queued while the callback reads the
                // last one, so a slow callback does not cost presented frames.
                let pool = shm.create_pool(file.as_fd(), (len * 2) as i32, &qh, ());
                let buffers = [0, len].map(|offset| {
                    pool.create_buffer(offset as i32, width as i32, height as i32, stride as i32, format, &qh, ())
                });
                let _ = ready_tx.send(Ok(()));

                let capture_into = |buffer: &wl_buffer::WlBuffer| {
                    let request = session.create_frame(&qh, ());
                    request.attach_buffer(buffer);
                    request.damage_buffer(0, 0, width as i32, height as i32);
                    request.capture();
                    request
                };
                let red_first = matches!(format, wl_shm::Format::Xbgr8888 | wl_shm::Format::Abgr8888);
                let mut frame = CapturedFrame {
                    red_first,
                    width,
                    height,
                    stride,
                    data: vec![0; len as usize],
                    presented: None,
                    sequence: 0,
                };
                let mut count = 0u64;
                let mut current = 0;
                let mut request = capture_into(&buffers[current]);
                while !state.stopped {
                    let finished =
                        dispatch_until(&conn, &mut queue, &mut state, &flag, Duration::from_secs(3600), |s| {
                            s.result.is_some() || s.stopped
                        })?;
                    request.destroy();
                    let result = state.result.take();
                    let presented = state.presented.take();
                    if !finished {
                        break;
                    }
                    match result {
                        Some(Ok(())) => {
                            let done = current;
                            current ^= 1;
                            request = capture_into(&buffers[current]);
                            let _ = conn.flush();
                            file.read_exact_at(&mut frame.data, u64::from(len) * done as u64)
                                .map_err(|e| e.to_string())?;
                            frame.presented = presented;
                            frame.sequence = count;
                            on_frame(&frame);
                            count += 1;
                        }
                        Some(Err(why)) => return Err(why),
                        None => break,
                    }
                }
                session.destroy();
                source.destroy();
                for buffer in buffers {
                    buffer.destroy();
                }
                pool.destroy();
                let _ = conn.flush();
                Ok(count)
            };
            let result = run();
            if let Err(why) = &result {
                let _ = ready_tx.send(Err(why.clone()));
            }
            result
        });
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(Self { stop, thread: Some(thread) }),
            Ok(Err(why)) => Err(why),
            Err(_) => Err("capture did not start in 10 s".into()),
        }
    }

    /// Stops capturing and returns how many frames were delivered.
    pub fn stop(mut self) -> Result<u64, String> {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().map_or(Ok(0), |t| t.join().unwrap_or_else(|_| Err("capture thread panicked".into())))
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// One frame of the output as it is now.
pub fn screenshot(hyprland: &Hyprland) -> Result<CapturedFrame, String> {
    screenshot_socket(&hyprland.wayland_socket())
}

/// As [`screenshot`], on the compositor behind `socket`.
pub fn screenshot_socket(socket: &std::path::Path) -> Result<CapturedFrame, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let capture = Capture::start_socket(socket, move |frame| {
        let _ = tx.send(CapturedFrame { data: frame.data.clone(), ..*frame });
    })?;
    let frame = rx.recv_timeout(Duration::from_secs(5)).map_err(|_| "no frame in 5 s".to_string());
    capture.stop()?;
    frame
}
