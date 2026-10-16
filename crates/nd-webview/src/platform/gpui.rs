//! The GPUI presenter (Linux, Windows, and macOS under `ND_BACKEND=gpui`). A page's engine
//! draws it off-screen and builds the element that shows it ([`EngineView::element`]), so the
//! page is part of the window's own frame: laid out, clipped and covered by overlays like any
//! other element. This presenter keeps which page each node shows, where its events go, and
//! parks the page when the node goes away.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
#[cfg(not(target_os = "macos"))]
use std::time::Duration;

use nd_backend_gpui::GpuiBackend;
use nd_backend_gpui::custom::{CustomCx, CustomWidget};
use nd_core::NodeId;
use nd_gpui::gpui::{self, AnyElement, IntoElement, ParentElement, Styled, div};
#[cfg(not(target_os = "macos"))]
use nd_ui::App;

use crate::element::{WebViewProps, WebViewWidget};
use crate::engine::{EngineView, PageState, Target};
use crate::events::LoadFailed;
use crate::handle::WebView;

thread_local! {
    /// The backend of the last window that showed a page: where [`defer`] and [`after`] run.
    static BACKEND: RefCell<Option<GpuiBackend>> = const { RefCell::new(None) };
}

pub(crate) fn register() {
    nd_backend_gpui::custom::register::<WebViewWidget>(Presenter::default());
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn defer(f: Box<dyn FnOnce(&mut App)>) {
    let backend = BACKEND.with(|b| b.borrow().clone());
    match backend {
        Some(backend) => backend.defer(f),
        None => tracing::warn!("web view: an answer arrived before any window showed a page; dropped"),
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn after(delay: Duration, f: impl FnOnce() + 'static) {
    let app = BACKEND.with(|b| b.borrow().as_ref().and_then(GpuiBackend::async_app));
    if let Some(app) = app {
        app.spawn(async move |cx| {
            cx.background_executor().timer(delay).await;
            f();
        })
        .detach();
    }
}

#[derive(Default)]
struct Presenter {
    nodes: RefCell<HashMap<NodeId, Mounted>>,
}

/// The page a node shows, and the engine view it mounted.
struct Mounted {
    page: WebView,
    props: WebViewProps,
    engine: Option<Rc<dyn EngineView>>,
    error: Option<String>,
}

impl Mounted {
    fn mount(props: &WebViewProps, cx: &CustomCx<'_>) -> Self {
        let page = props.page.clone();
        let shared = page.0.shared.clone();
        let backend = cx.backend().clone();
        *shared.target.borrow_mut() = Some(Target { emit: cx.emitter(), wake: Rc::new(move || backend.wake()) });
        *shared.props.borrow_mut() = Some(props.clone());
        *page.0.mounts.borrow_mut() += 1;
        let mut mounted = Mounted { page: page.clone(), props: props.clone(), engine: None, error: None };
        match page.view() {
            Ok(engine) => {
                engine.set_attached(true);
                engine.apply(props, None);
                mounted.engine = Some(engine);
                // The element that shows a parked page hears where it is.
                let state = page.state();
                let p = page.page();
                *shared.state.borrow_mut() = PageState { zoom: state.zoom, ..PageState::default() };
                p.report_url(&state.url);
                p.report_title(&state.title);
                p.report_loading(state.loading);
                p.report_back(state.can_go_back);
                p.report_forward(state.can_go_forward);
                p.report_progress(state.progress);
            }
            Err(error) => {
                page.page().emit(LoadFailed { url: props.url.clone().unwrap_or_default(), error: error.clone() });
                mounted.error = Some(error);
            }
        }
        mounted
    }

    fn update(&mut self, props: &WebViewProps) {
        if *props == self.props {
            return;
        }
        *self.page.0.shared.props.borrow_mut() = Some(props.clone());
        if let Some(engine) = &self.engine {
            engine.apply(props, Some(&self.props));
        }
        self.props = props.clone();
    }

    /// Gives the page up: it parks, unless a later mount has taken it over already.
    fn unmount(self) {
        let shared = &self.page.0.shared;
        let still_ours = {
            let mut mounts = self.page.0.mounts.borrow_mut();
            *mounts = mounts.saturating_sub(1);
            *mounts == 0
        };
        if still_ours {
            *shared.target.borrow_mut() = None;
            *shared.props.borrow_mut() = None;
            if let Some(engine) = &self.engine {
                engine.set_attached(false);
            }
        }
    }
}

impl CustomWidget for Presenter {
    fn element(&self, cx: &CustomCx<'_>, window: &mut gpui::Window) -> AnyElement {
        BACKEND.with(|b| *b.borrow_mut() = Some(cx.backend().clone()));
        let props = cx.props().downcast_ref::<WebViewProps>().expect("webview node needs WebViewProps").clone();
        let (engine, error) = {
            let mut nodes = self.nodes.borrow_mut();
            let same = nodes.get(&cx.node()).is_some_and(|m| m.page == props.page);
            if same {
                nodes.get_mut(&cx.node()).expect("checked").update(&props);
            } else {
                // A different page under the same key: a fresh mount.
                if let Some(old) = nodes.remove(&cx.node()) {
                    old.unmount();
                }
                nodes.insert(cx.node(), Mounted::mount(&props, cx));
            }
            let mounted = &nodes[&cx.node()];
            (mounted.engine.clone(), mounted.error.clone())
        };
        if let Some(element) = engine.as_ref().and_then(|e| e.element(cx, window)) {
            return element;
        }
        let reason =
            error.unwrap_or_else(|| format!("the {} engine cannot draw in this window", props.page.engine().name()));
        let theme = cx.theme();
        let mut notice = div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p(gpui::px(16.))
            .text_color(theme.text.secondary)
            .child(reason);
        gpui::Refineable::refine(notice.style(), &props.style);
        notice.into_any_element()
    }

    fn released(&self, node: NodeId) {
        let mounted = self.nodes.borrow_mut().remove(&node);
        if let Some(mounted) = mounted {
            mounted.unmount();
        }
    }
}
