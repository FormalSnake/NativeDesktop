//! `NativeSurface` on AppKit (ARCHITECTURE.md 8): the platform view is an ordinary subview in the
//! retained tree, framed in the transaction like its siblings, so it resizes in the same Core
//! Animation commit as the window's content. Overlays are separate windows above it
//! (`crate::overlay`). The minimal web host is a `WKWebView`; the full API is M8's.

use nd_ui::{SurfaceProps, SurfaceSource};
use objc2::MainThreadOnly;
use objc2::rc::Retained;
use objc2_app_kit::NSView;
use objc2_foundation::{MainThreadMarker, NSRect, NSString, NSURL, NSURLRequest};
use objc2_web_kit::{WKUserScript, WKUserScriptInjectionTime, WKWebView, WKWebViewConfiguration};

use crate::look::{Color, ViewLook};
use crate::views::NdLayerView;

pub(crate) enum SurfaceNative {
    Web {
        view: Retained<WKWebView>,
        props: Box<SurfaceProps>,
    },
    /// The GPU test pattern's stand-in: AppKit has no GPU surface host before the embedded GPUI
    /// surface patch lands, so it is a plain layer.
    Pattern(Retained<NdLayerView>),
}

fn load(view: &WKWebView, source: &SurfaceSource) {
    match source {
        SurfaceSource::Url(url) => {
            if let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) {
                unsafe { view.loadRequest(&NSURLRequest::requestWithURL(&url)) };
            }
        }
        SurfaceSource::Html(html) => {
            unsafe { view.loadHTMLString_baseURL(&NSString::from_str(html), None) };
        }
        SurfaceSource::TestPattern => {}
    }
}

impl SurfaceNative {
    pub fn new(mtm: MainThreadMarker, props: &SurfaceProps) -> Self {
        if props.source == SurfaceSource::TestPattern {
            let look = ViewLook { background: Some(Color::Rgba(0.2, 0.45, 0.9, 1.)), ..ViewLook::default() };
            return SurfaceNative::Pattern(NdLayerView::new(mtm, &look));
        }
        let config = unsafe { WKWebViewConfiguration::new(mtm) };
        if props.count_input {
            let script = unsafe {
                WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                    WKUserScript::alloc(mtm),
                    &NSString::from_str(nd_ui::surface::COUNTING_SCRIPT),
                    WKUserScriptInjectionTime::AtDocumentStart,
                    true,
                )
            };
            unsafe { config.userContentController().addUserScript(&script) };
        }
        let view = unsafe { WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), NSRect::ZERO, &config) };
        load(&view, &props.source);
        SurfaceNative::Web { view, props: Box::new(props.clone()) }
    }

    pub fn view(&self) -> &NSView {
        match self {
            SurfaceNative::Web { view, .. } => view,
            SurfaceNative::Pattern(view) => view,
        }
    }

    /// False when the change needs a new native view (web page to pattern or back).
    pub fn update(&mut self, new: &SurfaceProps) -> bool {
        match self {
            SurfaceNative::Web { view, props } if new.source != SurfaceSource::TestPattern => {
                if new.count_input != props.count_input {
                    return false;
                }
                if new.source != props.source {
                    load(view, &new.source);
                }
                **props = new.clone();
                true
            }
            SurfaceNative::Pattern(_) => new.source == SurfaceSource::TestPattern,
            SurfaceNative::Web { .. } => false,
        }
    }

    /// The input counts the page reported, when it counts.
    pub fn value(&self) -> Option<String> {
        match self {
            SurfaceNative::Web { view, props } if props.count_input => unsafe { view.title() }.map(|t| t.to_string()),
            _ => None,
        }
    }
}
