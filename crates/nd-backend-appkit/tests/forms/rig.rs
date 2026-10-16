//! What every forms suite shares: one app on the AppKit backend, a window per suite, node and
//! view lookups by test id, and a pump that drains native events the way the host does.
#![allow(dead_code)]

use std::cell::RefCell;
use std::time::Duration;

use nd_backend_appkit::objc2::rc::Retained;
use nd_backend_appkit::objc2_app_kit::NSView;
use nd_backend_appkit::objc2_foundation::MainThreadMarker;
use nd_backend_appkit::{AppKitBackend, NdAppKitPlatform, NdWindow, WindowHost};
use nd_ui::prelude::*;
use nd_ui::semantics::SemanticAction;
use nd_ui::{App, NodeId, SemanticNode, Size, WindowId};

pub struct Checks(pub u32);

impl Checks {
    pub fn check(&mut self, ok: bool, what: &str) {
        println!("{} {what}", if ok { "ok  " } else { "FAIL" });
        if !ok {
            self.0 += 1;
        }
    }
}

pub struct Rig {
    pub platform: NdAppKitPlatform,
    pub backend: AppKitBackend,
    pub app: RefCell<App>,
}

/// One suite's window; closing it is the suite's last step.
pub struct Opened<V: 'static> {
    pub handle: WindowHandle<V>,
    pub window: Retained<NdWindow>,
    _host: WindowHost,
}

impl<V: 'static> Opened<V> {
    pub fn id(&self) -> WindowId {
        self.handle.id()
    }
}

impl Rig {
    pub fn new() -> Self {
        let mtm = MainThreadMarker::new().expect("main thread");
        let platform = NdAppKitPlatform::new(mtm);
        let backend = AppKitBackend::new(mtm);
        backend.on_wake(|| {});
        let app = App::with_backend(Box::new(backend.clone()));
        Self { platform, backend, app: RefCell::new(app) }
    }

    pub fn open<V: Render>(&self, title: &str, build: impl FnOnce(&mut Context<V>) -> V) -> Opened<V> {
        let handle = self.app.borrow_mut().open_window(WindowOptions::main(title), |_, cx| cx.new(build));
        let window = self.platform.open_window(title, 480., 640.);
        let host = WindowHost::new(&self.backend, handle.id(), &window);
        self.platform.show(&window);
        let mut app = self.app.borrow_mut();
        app.resize_window(handle.id(), Size { width: 480., height: 640. });
        app.frame();
        drop(app);
        Opened { handle, window, _host: host }
    }

    pub fn close<V>(&self, opened: Opened<V>) {
        self.app.borrow_mut().close_window(opened.id());
        opened.window.close();
    }

    /// Runs the main loop briefly, then drains native events and renders, as the host does.
    pub fn pump(&self) {
        self.platform.pump(Duration::from_millis(20), Duration::from_millis(300));
        let mut app = self.app.borrow_mut();
        app.drain_events();
        app.frame();
    }

    pub fn update<V: 'static, R>(&self, opened: &Opened<V>, f: impl FnOnce(&mut V, &mut Context<V>) -> R) -> R {
        let mut app = self.app.borrow_mut();
        let r = opened.handle.root().update(&mut app, f);
        app.frame();
        r
    }

    pub fn read<V: 'static, R>(&self, opened: &Opened<V>, f: impl FnOnce(&V) -> R) -> R {
        let app = self.app.borrow();
        f(opened.handle.root().read(&app))
    }

    pub fn node(&self, window: WindowId, test_id: &str) -> Option<SemanticNode> {
        let app = self.app.borrow();
        let tree = app.semantic_window(window)?;
        tree.root.iter().find(|n| n.test_id.as_deref() == Some(test_id)).cloned()
    }

    pub fn node_id(&self, window: WindowId, test_id: &str) -> NodeId {
        self.node(window, test_id).unwrap_or_else(|| panic!("no node with test id {test_id}")).id
    }

    /// The NSView the backend holds for the node with `test_id`.
    pub fn view(&self, window: WindowId, test_id: &str) -> Retained<NSView> {
        let id = self.node_id(window, test_id);
        self.backend.view(id).unwrap_or_else(|| panic!("{test_id} has no view"))
    }

    /// Focuses the element keyed `id`, as `Window::focus` does, and renders.
    pub fn focus_id(&self, window: WindowId, id: &str) -> bool {
        let mut app = self.app.borrow_mut();
        let focused = app.focus_id(window, id);
        app.frame();
        focused
    }

    pub fn perform(&self, window: WindowId, test_id: &str, action: SemanticAction) -> Result<(), String> {
        let id = self.node_id(window, test_id);
        let result = self.app.borrow_mut().perform(id, &action).map_err(|e| e.to_string());
        self.pump();
        result
    }
}
