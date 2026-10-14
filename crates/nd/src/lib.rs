//! The one crate apps depend on: `use nd::prelude::*;`.
//!
//! The vocabulary, retained tree and layout are `nd-ui`'s, on AppKit (macOS) and GPUI
//! (elsewhere, and on macOS with the `gpui` feature and `ND_BACKEND=gpui`); the theme is
//! `nd-theme`'s; the chrome intents, `glass()`, menus and dialogs are `nd-components`'.

/// The GPUI backend, whichever crate brings it on this target (Cargo.toml says why).
#[cfg(all(target_os = "macos", feature = "gpui"))]
pub(crate) use nd_backend_gpui as gpui_backend;
#[cfg(all(target_os = "linux", feature = "runtime"))]
pub(crate) use nd_os_linux::nd_backend_gpui as gpui_backend;
#[cfg(all(target_arch = "wasm32", feature = "runtime"))]
pub(crate) use nd_os_web::nd_backend_gpui as gpui_backend;
#[cfg(all(windows, feature = "runtime"))]
pub(crate) use nd_os_windows::nd_backend_gpui as gpui_backend;

#[cfg(feature = "audit")]
pub mod audit;
#[cfg(feature = "build")]
pub mod build;
#[cfg(feature = "runtime")]
pub mod chrome;
#[cfg(feature = "runtime")]
pub mod clipboard;
#[cfg(feature = "runtime")]
mod controls;
#[cfg(feature = "runtime")]
pub mod files;
#[cfg(feature = "runtime")]
pub mod media;
#[cfg(feature = "runtime")]
pub mod native;
#[cfg(feature = "runtime")]
mod runtime;
#[cfg(feature = "runtime")]
pub mod system;
#[cfg(feature = "runtime")]
pub mod theme;
#[cfg(feature = "runtime")]
pub use runtime::*;

#[cfg(feature = "runtime")]
pub use nd_components as components;
#[cfg(feature = "runtime")]
pub use nd_core as core;
/// Layered settings and `Store<T>`. `#[derive(Settings)]` finds this path on its own; a settings
/// type's `JsonSchema` derive needs `#[schemars(crate = "nd::schemars")]`.
#[cfg(feature = "runtime")]
pub use nd_settings as settings;
#[cfg(feature = "runtime")]
pub use nd_settings::schemars;
/// Icons by Lucide name, and `icons::register` for an app's own.
#[cfg(feature = "runtime")]
pub use nd_icons as icons;
/// Capability checks: `nd::platform::has(Feature::MultiWindow)`.
#[cfg(feature = "runtime")]
pub use nd_core::platform;
/// Clocks that also work in a browser.
#[cfg(feature = "runtime")]
pub use nd_core::time;
#[cfg(feature = "runtime")]
pub use nd_macros::unavailable;
#[cfg(feature = "terminal")]
pub use nd_terminal as terminal;
#[cfg(feature = "runtime")]
pub use nd_ui::{actions, backend, dispatch, events, transaction};
#[cfg(feature = "runtime")]
pub use nd_update as update;
#[cfg(feature = "runtime")]
pub use nd_virtual as virtual_list;
/// `webview(id, &page)` and [`webview::WebView`] (nd-webview).
#[cfg(feature = "webview")]
pub use nd_webview as webview;

#[cfg(all(feature = "runtime", target_os = "macos"))]
pub use nd_backend_appkit as appkit;

#[cfg(feature = "test")]
pub use nd_macros::test;

#[doc(hidden)]
#[cfg(all(feature = "test", target_arch = "wasm32"))]
pub mod __private {
    pub use wasm_bindgen_test;
}

/// Tier 1 vocabulary for app tests: `TestApp` over the recording `TestBackend`.
#[cfg(feature = "test")]
pub mod test {
    pub use nd_core::semantic::{Role, SemanticNode};
    pub use nd_ui::semantics::{ActionError, SemanticAction};
    pub use nd_ui::test_app::TestApp;
    pub use nd_ui::test_backend::TestBackend;
    pub use nd_ui::{ButtonProps, NodeId, Size, TextProps};

    use nd_core::WindowId;

    /// `window`'s semantic tree once the app has settled (a window opened on the app directly
    /// has not run its first frame yet).
    pub fn tree(cx: &mut TestApp, window: WindowId) -> SemanticNode {
        cx.run_until_idle();
        cx.semantic_window(window).unwrap_or_else(|| panic!("{window:?} is not open")).root
    }

    /// The node in `window` whose explicit id is `test_id`.
    pub fn by_test_id(cx: &mut TestApp, window: WindowId, test_id: &str) -> Option<SemanticNode> {
        tree(cx, window).iter().find(|n| n.test_id.as_deref() == Some(test_id)).cloned()
    }

    /// Every name in `window`, space separated.
    pub fn text(cx: &mut TestApp, window: WindowId) -> String {
        tree(cx, window).subtree_text()
    }

    /// Clicks the node `test_id` as assistive technology would, then settles.
    pub fn click(cx: &mut TestApp, window: WindowId, test_id: &str) {
        let node = by_test_id(cx, window, test_id).unwrap_or_else(|| panic!("no node `{test_id}` in {window:?}"));
        if let Err(e) = cx.perform(node.id, &SemanticAction::Click) {
            panic!("clicking `{test_id}`: {e:?}");
        }
        cx.run_until_idle();
    }
}
