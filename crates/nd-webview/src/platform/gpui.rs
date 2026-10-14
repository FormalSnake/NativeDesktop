//! The GPUI presenter (Linux, Windows, and macOS under `ND_BACKEND=gpui`). WPE WebKit and
//! WebView2 are not written yet, so the node shows why instead of a page.

use nd_backend_gpui::custom::{CustomCx, CustomWidget};
use nd_gpui::gpui::{self, AnyElement, IntoElement, ParentElement, Styled, div};

use crate::element::{WebViewProps, WebViewWidget};

pub(crate) fn register() {
    nd_backend_gpui::custom::register::<WebViewWidget>(Notice);
}

struct Notice;

impl CustomWidget for Notice {
    fn element(&self, cx: &CustomCx<'_>, _window: &mut gpui::Window) -> AnyElement {
        let props = cx.props().downcast_ref::<WebViewProps>().expect("webview node needs WebViewProps");
        let reason = match props.page.view() {
            Ok(_) => "this backend cannot show a web page".to_owned(),
            Err(error) => error,
        };
        let theme = cx.theme();
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p(gpui::px(16.))
            .text_color(theme.text.secondary)
            .child(reason)
            .into_any_element()
    }
}
