//! `shelf()`: a horizontal row of cards that scrolls sideways (a carousel), with paging arrows
//! that show while the pointer is over it and snapping to the cards' leading edges.
//!
//! Each shelf keeps its own position in a [`ShelfHandle`] the app holds per section (the
//! youtubemusic page kept one `ScrollHandle` per section), so a re-render never resets it and
//! the app can page a shelf from elsewhere (a "See all" header's arrows). On AppKit the shelf
//! is an `NSScrollView` with the OS's momentum and rubber band, arrows as borderless system
//! buttons over its edges; on GPUI backends a drawn horizontal scroll area.
//!
//! Only sideways motion scrolls the shelf: a vertical wheel over it scrolls the page around it.

use std::cell::RefCell;
use std::rc::Rc;

use nd_core::Rect;
use nd_gpui::gpui::Overflow;
use nd_ui::prelude::*;
use nd_ui::{ContainerElement, DispatchProps, InteractiveElement, Key, Role, StyleRefinement};

/// When a shelf shows its paging arrows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ShelfArrows {
    /// While the pointer is over the shelf, as the App Store and Music shelves do.
    #[default]
    Hover,
    Always,
    Never,
}

/// A scroll the app asked for, applied once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShelfScroll {
    /// One page back (`-1`) or forward (`1`): the viewport less one card of context.
    Page(i32),
    /// Brings card `index` to the leading edge.
    Item(usize),
    Offset(f32),
}

#[derive(Debug, Default)]
struct Core {
    offset: f32,
    max_offset: f32,
    viewport: f32,
    request: Option<(u64, ShelfScroll)>,
    seq: u64,
}

/// One shelf's position. Keep it in the view's state, one per shelf; the backend writes the
/// position back as the user scrolls.
#[derive(Clone, Debug, Default)]
pub struct ShelfHandle(Rc<RefCell<Core>>);

impl PartialEq for ShelfHandle {
    /// Equal handles with no new request: a re-render sends no update.
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl ShelfHandle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks for `scroll`; it applies on the next render of the shelf.
    pub fn scroll(&self, scroll: ShelfScroll) {
        let mut core = self.0.borrow_mut();
        core.seq += 1;
        core.request = Some((core.seq, scroll));
    }

    pub fn page(&self, direction: i32) {
        self.scroll(ShelfScroll::Page(direction.signum()));
    }

    pub fn scroll_to_item(&self, index: usize) {
        self.scroll(ShelfScroll::Item(index));
    }

    /// The scroll offset from the leading edge, in points.
    pub fn offset(&self) -> f32 {
        self.0.borrow().offset
    }

    pub fn can_page_back(&self) -> bool {
        self.0.borrow().offset > 1.
    }

    pub fn can_page_forward(&self) -> bool {
        let core = self.0.borrow();
        core.max_offset > 1. && core.offset < core.max_offset - 1.
    }

    /// The request a backend has not applied yet, and its sequence number.
    pub fn pending(&self) -> Option<(u64, ShelfScroll)> {
        self.0.borrow().request
    }

    /// The backend applied the request numbered `seq`.
    pub fn applied(&self, seq: u64) {
        let mut core = self.0.borrow_mut();
        if core.request.is_some_and(|(s, _)| s == seq) {
            core.request = None;
        }
    }

    /// The backend reports where the shelf is.
    pub fn report(&self, offset: f32, max_offset: f32, viewport: f32) {
        let mut core = self.0.borrow_mut();
        core.offset = offset;
        core.max_offset = max_offset.max(0.);
        core.viewport = viewport;
    }

    /// The request's sequence number, which the props compare so a new request reaches the
    /// backend as a prop change.
    pub fn seq(&self) -> u64 {
        self.0.borrow().seq
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShelfProps {
    pub handle: ShelfHandle,
    /// The handle's request count when the shelf rendered.
    pub seq: u64,
    pub arrows: ShelfArrows,
    /// Settles on a card's leading edge after a scroll.
    pub snap: bool,
    /// Accessible name ("Recently played").
    pub label: SharedString,
    /// Points the shelf reaches past its box on either side, see [`Shelf::bleed`].
    pub bleed: f32,
    pub style: StyleRefinement,
}

/// The shelf's position changed in a way the app may show: which arrows can page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShelfScrolled {
    pub offset: f32,
    pub can_page_back: bool,
    pub can_page_forward: bool,
}

/// Widget kind of `shelf`.
pub struct ShelfKind;

pub struct Shelf(ContainerElement<ShelfProps>);

fn describe(props: &nd_ui::backend::Props) -> (Role, String) {
    let name = props.downcast_ref::<ShelfProps>().map(|p| p.label.to_string()).unwrap_or_default();
    (Role::ScrollArea, name)
}

/// A sideways-scrolling row of cards. `handle` keeps its position across renders.
pub fn shelf(id: impl Into<SharedString>, handle: &ShelfHandle) -> Shelf {
    nd_ui::semantics::describe_widget::<ShelfKind>(describe);
    let props = ShelfProps {
        handle: handle.clone(),
        seq: handle.seq(),
        arrows: ShelfArrows::Hover,
        snap: true,
        label: SharedString::default(),
        bleed: 0.,
        style: StyleRefinement::default(),
    };
    let mut el = ContainerElement::new::<ShelfKind>(Key::Id(id.into()), props).flex().flex_row().w_full();
    // Its content is wider than it is: Taffy must not size the shelf to its cards.
    el.style().overflow.x = Some(Overflow::Scroll);
    Shelf(el)
}

impl Shelf {
    pub fn arrows(mut self, arrows: ShelfArrows) -> Self {
        self.0.props_mut().arrows = arrows;
        self
    }

    /// Stops wherever the scroll ends instead of on a card's edge.
    pub fn free_scrolling(mut self) -> Self {
        self.0.props_mut().snap = false;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.0.props_mut().label = label.into();
        self
    }

    /// Runs the shelf `points` past its box on both sides, out over the page's margins, with
    /// its cards inset by as much: at rest the first card lines up with the page's headings,
    /// and scrolled, cards pass to the content's edges instead of being cut at the margin, as
    /// Music's and the App Store's shelves do. The paging arrows sit in the margins. The shelf
    /// takes its width from its parent, so the parent lays it out as a block or a column.
    pub fn bleed(mut self, points: f32) -> Self {
        self.0.props_mut().bleed = points;
        self.0 = self.0.w_auto().ml(px(-points)).mr(px(-points)).pl(px(points)).pr(px(points));
        self
    }

    pub fn on_scroll(self, f: impl Fn(&ShelfScrolled, &mut Window, &mut App) + 'static) -> Self {
        Shelf(self.0.on(f))
    }
}

impl Styled for Shelf {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl ParentElement for Shelf {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.0.extend(elements);
    }
}

impl FluentBuilder for Shelf {}

impl InteractiveElement for Shelf {
    fn dispatch_props(&mut self) -> &mut DispatchProps {
        self.0.dispatch_props()
    }
}

impl IntoElement for Shelf {
    fn into_any_element(mut self) -> AnyElement {
        let style = self.0.style().clone();
        self.0.props_mut().style = style;
        self.0.into_any_element()
    }
}

/// The offset a scroll request lands on, from the cards' frames along the shelf (`x` and
/// width, in content coordinates, in order) and the shelf's `inset` before the first card.
pub fn scroll_target(
    request: ShelfScroll,
    offset: f32,
    viewport: f32,
    max_offset: f32,
    cards: &[Rect],
    inset: f32,
) -> f32 {
    let target = match request {
        ShelfScroll::Offset(x) => x,
        ShelfScroll::Item(i) => cards.get(i).map_or(offset, |c| c.origin.x - inset),
        ShelfScroll::Page(direction) => {
            // Keep one card of context: the viewport less the widest card.
            let card = cards.iter().map(|c| c.size.width).fold(0f32, f32::max);
            let step = (viewport - card).max(card).max(1.);
            let raw = offset + step * direction.signum() as f32;
            snap_offset(raw, cards, inset, max_offset, direction.signum())
        }
    };
    target.clamp(0., max_offset.max(0.))
}

/// The card edge nearest `offset`, toward `direction` when it is not 0. The end of the shelf
/// is always a valid stop.
pub fn snap_offset(offset: f32, cards: &[Rect], inset: f32, max_offset: f32, direction: i32) -> f32 {
    let stops = cards.iter().map(|c| (c.origin.x - inset).clamp(0., max_offset.max(0.))).chain([max_offset.max(0.)]);
    let pick = |best: Option<f32>, stop: f32| -> Option<f32> {
        let ok = match direction {
            d if d > 0 => stop <= offset + 0.5,
            d if d < 0 => stop >= offset - 0.5,
            _ => true,
        };
        if !ok {
            return best;
        }
        match best {
            Some(b) if (b - offset).abs() <= (stop - offset).abs() => Some(b),
            _ => Some(stop),
        }
    };
    stops.fold(None, pick).unwrap_or(offset.clamp(0., max_offset.max(0.)))
}
