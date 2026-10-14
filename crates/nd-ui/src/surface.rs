//! `NativeSurface` (ARCHITECTURE.md 8): a platform view that joins the window. nd lays it out like
//! any leaf and the backend places it in the frame's transaction, so it moves and resizes in
//! lockstep with the window. Overlays always sit above it and its input stops at their edges
//! (6.3).
//!
//! What it shows is a [`SurfaceSource`]: a web page (WKWebView on macOS today; WebView2 and WPE
//! arrive with M8) or a test pattern drawn on the GPU, which the overlay and resize gates use
//! where no web engine is wired up yet.
//!
//! With `.count_input()` its semantic value counts the pointer input it received, as
//! `down=N up=N move=N wheel=N`, so a test can prove that nothing reached it through an overlay.
//! A web surface counts inside the page (a user script reporting through the title), which is
//! what the page actually saw.

use nd_gpui::gpui::prelude::FluentBuilder;
use nd_gpui::gpui::{SharedString, StyleRefinement, Styled};

use crate::element::{AnyElement, DispatchProps, InteractiveElement, IntoElement, Key, NativeElement};

/// The widget kind of every surface node.
pub struct NativeSurface;

#[derive(Clone, Debug, PartialEq)]
pub enum SurfaceSource {
    /// A page by URL.
    Url(SharedString),
    /// A page from an HTML string.
    Html(SharedString),
    /// Stripes drawn on the GPU, counting the input that reaches them.
    TestPattern,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceProps {
    pub source: SurfaceSource,
    pub count_input: bool,
    /// The element's style, for backends that lay out themselves (GPUI).
    pub style: StyleRefinement,
}

/// Counts of the pointer input a surface received, in its semantic value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputCounts {
    pub down: u32,
    pub up: u32,
    pub moved: u32,
    pub wheel: u32,
}

impl InputCounts {
    pub fn total(&self) -> u32 {
        self.down + self.up + self.moved + self.wheel
    }

    pub fn parse(value: &str) -> Option<Self> {
        let mut counts = Self::default();
        for part in value.split_whitespace() {
            let (key, n) = part.split_once('=')?;
            let n: u32 = n.parse().ok()?;
            match key {
                "down" => counts.down = n,
                "up" => counts.up = n,
                "move" => counts.moved = n,
                "wheel" => counts.wheel = n,
                _ => {}
            }
        }
        Some(counts)
    }
}

impl std::fmt::Display for InputCounts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "down={} up={} move={} wheel={}", self.down, self.up, self.moved, self.wheel)
    }
}

/// The script a web surface runs so its counts reach the semantic tree through the page title.
pub const COUNTING_SCRIPT: &str = "(function(){var c={down:0,up:0,move:0,wheel:0};\
function t(){document.title='down='+c.down+' up='+c.up+' move='+c.move+' wheel='+c.wheel;}\
addEventListener('mousedown',function(){c.down++;t();},true);\
addEventListener('mouseup',function(){c.up++;t();},true);\
addEventListener('mousemove',function(){c.move++;t();},true);\
addEventListener('wheel',function(){c.wheel++;t();},{capture:true,passive:true});\
addEventListener('DOMContentLoaded',t);t();})();";

pub struct Surface(NativeElement<SurfaceProps>);

/// A native surface keyed by `id`; stateful, so the id keeps its view (and page) across renders.
pub fn native_surface(id: impl Into<SharedString>, source: SurfaceSource) -> Surface {
    Surface(NativeElement::new::<NativeSurface>(
        Key::Id(id.into()),
        SurfaceProps { source, count_input: false, style: StyleRefinement::default() },
    ))
}

/// A web page in a native surface.
pub fn webview(id: impl Into<SharedString>, url: impl Into<SharedString>) -> Surface {
    native_surface(id, SurfaceSource::Url(url.into()))
}

impl Surface {
    pub fn props(&self) -> &SurfaceProps {
        self.0.props()
    }

    /// Reports the pointer input that reaches the surface in its semantic value.
    pub fn count_input(mut self) -> Self {
        self.0.props_mut().count_input = true;
        self
    }
}

impl Styled for Surface {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl FluentBuilder for Surface {}

impl InteractiveElement for Surface {
    fn dispatch_props(&mut self) -> &mut DispatchProps {
        self.0.dispatch_props()
    }
}

impl IntoElement for Surface {
    fn into_any_element(mut self) -> AnyElement {
        let style = self.0.style().clone();
        self.0.props_mut().style = style;
        self.0.into_any_element()
    }
}
