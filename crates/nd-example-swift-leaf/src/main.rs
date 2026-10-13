//! Shows the SwiftUI `Gauge` leaf of `NDSwift` inside an NSWindow. Prints `ND_WINDOW_ID=<n>`
//! once the window is up and quits after `ND_EXAMPLE_SECONDS` (default 30).

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    use nd::appkit::SwiftGauge;
    use nd::appkit::objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow, NSWindowStyleMask,
    };
    use nd::appkit::objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

    let gauge = SwiftGauge::new(mtm, 72.0, (0.0, 100.0), "Disk");
    let size = gauge.measure();
    let frame = NSRect::new(NSPoint::new(200.0, 200.0), NSSize::new(size.width + 80.0, size.height + 80.0));
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            mtm.alloc(),
            frame,
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str("Swift leaf"));
    let content = window.contentView().expect("content view");
    let origin = NSPoint::new((frame.size.width - size.width) / 2.0, (frame.size.height - size.height) / 2.0);
    gauge.view().setFrame(NSRect::new(origin, size));
    content.addSubview(gauge.view());
    window.makeKeyAndOrderFront(None);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    println!("ND_WINDOW_ID={}", window.windowNumber());

    let seconds = std::env::var("ND_EXAMPLE_SECONDS").ok().and_then(|s| s.parse().ok()).unwrap_or(30);
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(seconds));
        std::process::exit(0);
    });
    app.run();
}
