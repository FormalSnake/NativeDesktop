//! `webview(id, &page)`: the element that shows a [`WebView`], and its handlers.

use nd_ui::actions::{FocusNext, FocusPrevious};
use nd_ui::prelude::*;
use nd_ui::semantics::{Semantics, WidgetSemantics};
use nd_ui::{App, InteractiveElement, Key, NativeElement, Role, StyleRefinement, Window};

use crate::events::*;
use crate::handle::WebView;
use crate::menu::{ContextMenuMode, MenuItem};

/// The widget kind backends register their web view presenter for.
pub struct WebViewWidget;

#[derive(Clone, Debug, PartialEq)]
pub struct WebViewProps {
    pub page: WebView,
    /// Declarative address: the page loads it when it differs from both the address it shows
    /// and the one it is loading, so feeding [`Navigate`] back into it never reloads.
    pub url: Option<String>,
    pub context_menu_mode: ContextMenuMode,
    pub context_menu_items: Vec<MenuItem>,
    /// Counts the pointer input the page sees into its title (`down=N up=N move=N wheel=N`), for
    /// the overlay leak gates.
    pub count_input: bool,
    /// The element's layout style, for backends that lay widgets out themselves.
    pub style: StyleRefinement,
}

impl WidgetSemantics for WebViewProps {
    fn semantics(&self) -> Semantics {
        let state = self.page.state();
        Semantics { role: Some(Role::Group), name: state.title, value: Some(state.url), ..Semantics::default() }
    }
}

/// Shows `page`. Mount a page in one element at a time; mounting it elsewhere moves it there
/// without a reload.
pub fn webview(id: impl Into<Key>, page: &WebView) -> WebViewElement {
    WebViewElement(NativeElement::described::<WebViewWidget>(
        id,
        WebViewProps {
            page: page.clone(),
            url: None,
            context_menu_mode: ContextMenuMode::Native,
            context_menu_items: Vec::new(),
            count_input: false,
            style: StyleRefinement::default(),
        },
    ))
}

pub struct WebViewElement(NativeElement<WebViewProps>);

macro_rules! handlers {
    ($($(#[$doc:meta])* $name:ident => $event:ty;)*) => {
        $(
            $(#[$doc])*
            pub fn $name(self, f: impl Fn(&$event, &mut Window, &mut App) + 'static) -> Self {
                Self(self.0.on(f))
            }
        )*
    };
}

impl WebViewElement {
    pub fn props(&self) -> &WebViewProps {
        self.0.props()
    }

    /// Loads `url` whenever it differs from where the page is.
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.0.props_mut().url = Some(url.into());
        self
    }

    pub fn context_menu_mode(mut self, mode: ContextMenuMode) -> Self {
        self.0.props_mut().context_menu_mode = mode;
        self
    }

    /// The app's items, appended to the engine's menu (or the only items, with
    /// [`ContextMenuMode::Suppress`] and the app's own menu).
    pub fn context_menu_items(mut self, items: Vec<MenuItem>) -> Self {
        self.0.props_mut().context_menu_items = items;
        self
    }

    pub fn count_input(mut self) -> Self {
        self.0.props_mut().count_input = true;
        self
    }

    handlers! {
        on_navigate => Navigate;
        on_title_changed => TitleChanged;
        on_loading_changed => LoadingChanged;
        on_focus_changed => FocusChanged;
        on_back_available => BackAvailable;
        on_forward_available => ForwardAvailable;
        on_load_progress => LoadProgress;
        on_load_failed => LoadFailed;
        on_render_process_gone => RenderProcessGone;
        on_new_window => NewWindow;
        on_popup_blocked => PopupBlocked;
        on_window_closed => WindowClosed;
        on_download_requested => DownloadRequested;
        on_download_updated => DownloadUpdated;
        on_script_message => ScriptMessage;
        on_scheme_request => SchemeRequest;
        on_cookies_changed => CookiesChanged;
        on_favicon_changed => FaviconChanged;
        on_find_result => FindResult;
        on_security_changed => SecurityChanged;
        on_link_hover => LinkHover;
        on_context_menu => ContextMenu;
        on_context_menu_item_clicked => ContextMenuItemClicked;
        on_audio_state_changed => AudioStateChanged;
        on_zoom_changed => ZoomChanged;
        on_fullscreen_changed => FullscreenChanged;
        on_permission_request => PermissionRequest;
        on_permission_request_dismissed => PermissionRequestDismissed;
        on_close_approved => CloseApproved;
        on_external_protocol => ExternalProtocol;
        on_chrome_command => ChromeCommand;
        on_extension_popup_opened => ExtensionPopupOpened;
        on_extension_popup_closed => ExtensionPopupClosed;
        on_extension_popup_resized => ExtensionPopupResized;
    }
}

impl Styled for WebViewElement {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl FluentBuilder for WebViewElement {}

impl InteractiveElement for WebViewElement {
    fn dispatch_props(&mut self) -> &mut nd_ui::DispatchProps {
        self.0.dispatch_props()
    }
}

impl IntoElement for WebViewElement {
    fn into_any_element(mut self) -> AnyElement {
        crate::platform::register_presenters();
        let style = self.0.style().clone();
        self.0.props_mut().style = style;
        // Focused, the page keeps Tab and Shift-Tab (focus moves between its own fields) and
        // Return (forms submit); nd's traversal resumes once focus leaves the page.
        let (next, previous) = (self.0.props().page.clone(), self.0.props().page.clone());
        self.0
            .focusable()
            .takes_return()
            .on_action(move |_: &FocusNext, _, _| next.forward_key())
            .on_action(move |_: &FocusPrevious, _, _| previous.forward_key())
            .into_any_element()
    }
}
