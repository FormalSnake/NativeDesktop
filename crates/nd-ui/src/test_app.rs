//! Tier 1 harness: an [`App`] over a [`TestBackend`].

use std::ops::{Deref, DerefMut};

use nd_core::{NodeId, Size};
use nd_gpui::gpui;

use crate::app::{App, Entity, Render};
use crate::test_backend::TestBackend;
use crate::window::{Window, WindowHandle, WindowOptions};

pub struct TestApp {
    app: App,
}

impl Default for TestApp {
    fn default() -> Self {
        Self::new()
    }
}

impl TestApp {
    pub fn new() -> Self {
        Self::with_backend(TestBackend::new())
    }

    pub fn with_backend(backend: TestBackend) -> Self {
        Self { app: App::with_backend(Box::new(backend)) }
    }

    pub fn backend(&self) -> &TestBackend {
        self.app.backend_as().expect("TestBackend")
    }

    pub fn backend_mut(&mut self) -> &mut TestBackend {
        self.app.backend_as_mut().expect("TestBackend")
    }

    /// Opens a window of `size` and runs its first frame.
    pub fn open_window<V: Render>(
        &mut self,
        size: Size,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> WindowHandle<V> {
        let options = WindowOptions::main("test").size(gpui::size(gpui::px(size.width), gpui::px(size.height)));
        let handle = self.app.open_window(options, build);
        self.app.run_until_idle();
        handle
    }

    /// Emits `event` from `node` as its native widget would, then settles.
    pub fn emit<E: 'static>(&mut self, node: NodeId, event: E) {
        self.app.events().emitter(node).emit(event);
        self.app.run_until_idle();
    }
}

impl Deref for TestApp {
    type Target = App;
    fn deref(&self) -> &App {
        &self.app
    }
}

impl DerefMut for TestApp {
    fn deref_mut(&mut self) -> &mut App {
        &mut self.app
    }
}
