//! Runs an `nd-ui` [`App`] on AppKit: one [`WindowHost`] per app window, frames inside AppKit's
//! layout pass (so a live resize presents the frame it laid out), the display clock waking
//! layout while the app has work, and native events drained on the run loop turn after they
//! were queued.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};

use nd_core::WindowId;
use nd_ui::App;
use objc2_foundation::MainThreadMarker;

use crate::{AppKitBackend, NdAppKitPlatform, WindowHost};

struct Host {
    app: RefCell<App>,
    windows: RefCell<BTreeMap<WindowId, WindowHost>>,
    platform: NdAppKitPlatform,
    backend: AppKitBackend,
}

impl Host {
    /// Opens a host for every app window that has none, and drops hosts of closed ones.
    fn sync_windows(self: &Rc<Self>) {
        let Ok(app) = self.app.try_borrow() else { return };
        let ids: Vec<WindowId> = app.window_ids().collect();
        let mut windows = self.windows.borrow_mut();
        windows.retain(|id, _| ids.contains(id));
        for id in ids {
            if windows.contains_key(&id) {
                continue;
            }
            let Some(window) = app.window(id) else { continue };
            let size = window.viewport_size();
            let ns_window = self.platform.open_window(window.title(), size.width.into(), size.height.into());
            let host = WindowHost::new(&self.backend, id, &ns_window);
            let this = Rc::downgrade(self);
            host.on_layout(move |size| {
                let Some(this) = this.upgrade() else { return };
                let Ok(mut app) = this.app.try_borrow_mut() else { return };
                app.resize_window(id, size);
                app.frame();
                drop(app);
                this.after_frame();
            });
            let this = Rc::downgrade(self);
            host.clock().on_frame(move |_| tick(&this));
            ns_window.center();
            self.platform.show(&ns_window);
            windows.insert(id, host);
        }
    }

    fn after_frame(self: &Rc<Self>) {
        let opened = self.app.borrow().window_ids().count() != self.windows.borrow().len();
        if opened {
            let this = self.clone();
            // A frame can open or close windows; hosts follow on the next turn, outside layout.
            crate::events::defer(move || this.sync_windows());
        }
        self.kick();
    }

    /// Starts the display clocks while the app has a frame to draw.
    fn kick(&self) {
        if self.app.try_borrow().is_ok_and(|app| app.needs_frame()) {
            for host in self.windows.borrow().values() {
                host.clock().set_animating(true);
            }
        }
    }
}

/// On each display refresh: a layout pass (where the frame runs) while there is work, else the
/// clock pauses.
fn tick(host: &Weak<Host>) {
    let Some(host) = host.upgrade() else { return };
    let busy = host.app.try_borrow().map_or(true, |app| app.needs_frame());
    for window in host.windows.borrow().values() {
        if busy {
            window.request_layout();
        } else {
            window.clock().set_animating(false);
        }
    }
}

/// Builds the app with `init` (which opens its windows) and runs AppKit's event loop. Never
/// returns; AppKit exits the process when the last window closes.
pub fn run(init: impl FnOnce(&mut App)) -> ! {
    let mtm = MainThreadMarker::new().expect("nd apps run on the main thread");
    let platform = NdAppKitPlatform::new(mtm);
    let backend = AppKitBackend::new(mtm);
    let mut app = App::with_backend(Box::new(backend.clone()));
    init(&mut app);
    let host = Rc::new(Host { app: RefCell::new(app), windows: RefCell::new(BTreeMap::new()), platform, backend });
    let weak = Rc::downgrade(&host);
    host.backend.on_wake(move || {
        let Some(host) = weak.upgrade() else { return };
        if let Ok(mut app) = host.app.try_borrow_mut() {
            app.drain_events();
        }
        host.kick();
    });
    host.sync_windows();
    host.platform.run();
    std::process::exit(0)
}
