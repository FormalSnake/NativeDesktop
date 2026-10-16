//! The GPUI presenter (Linux, Windows, the web, and macOS under `ND_BACKEND=gpui`): the grid
//! drawn with GPUI's quads, glyph sprites and images in one element, so it shares the window's
//! renderer and atlas. Rows are shaped by GPUI's text system (which caches layouts between
//! frames) and every glyph is placed at its own cell, so fallback faces never drift the grid.

use std::borrow::Cow;
use std::cell::{Cell as StdCell, RefCell};
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc;
use nd_backend_gpui::custom::{CustomCx, CustomWidget};
use nd_core::NodeId;
use nd_gpui::gpui::{
    self, AnyElement, App, Bounds, ClipboardItem, CursorStyle, DispatchPhase, Element, ElementId, FocusHandle, Font,
    FontFallbacks, FontStyle, FontWeight, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId,
    IntoElement, KeyDownEvent, KeyUpEvent, LayoutId, Modifiers, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, RenderImage, ScrollDelta, ScrollWheelEvent, SharedString, Style, TextRun, UTF16Selection, UnderlineStyle,
    Window, fill, point, px, size,
};
use nd_theme::Theme;
use nd_theme::color::with_alpha;

use crate::element::{FocusChanged, LinkActivated, TerminalProps, TerminalWidget, emit_session_events};
use crate::frame::{Attrs, CursorShape, Frame, Underline};
use crate::glyph::{self, Ink, SYMBOLS_FAMILY, SYMBOLS_FONT};
use crate::icons;
use crate::input::{Key, KeyAction, KeyInput, Mods, MouseButton, key_for_name, unshifted_us};
use crate::links::Link;
use crate::paint::{self, cell_colors};
use crate::session::{GridSize, TerminalSession, WakerGuard};
use crate::sprite;
use crate::theme::{Rgb, TerminalTheme};

/// Point size when neither the element nor the theme names one.
pub const DEFAULT_FONT_SIZE: f32 = 13.;

pub(crate) fn register() {
    nd_backend_gpui::custom::register::<TerminalWidget>(GpuiTerminal::default());
}

#[derive(Default)]
struct GpuiTerminal {
    nodes: RefCell<HashMap<NodeId, Rc<NodeState>>>,
    fonts_loaded: StdCell<bool>,
}

/// A cached mask image: glyph, device size, colour.
type MaskKey = (char, u32, u32, [u8; 4]);

/// What one terminal node keeps between frames.
struct NodeState {
    session: RefCell<TerminalSession>,
    focus: RefCell<Option<FocusHandle>>,
    focused: StdCell<bool>,
    /// The output waker and the task that redraws the window when it fires.
    waking: RefCell<Option<(WakerGuard, gpui::Task<()>)>>,
    /// Input method text being composed, drawn at the cursor until committed.
    marked: RefCell<Option<String>>,
    /// The last key-down's key, so text the input handler commits for it encodes as that key.
    pending_key: RefCell<Option<(Key, Mods)>>,
    held: StdCell<Option<MouseButton>>,
    hover: RefCell<Option<Link>>,
    sprites: RefCell<HashMap<MaskKey, Arc<RenderImage>>>,
    layout: StdCell<Option<GridLayout>>,
    /// Whether the grid's font has a glyph for an icon codepoint, by font family and codepoint.
    coverage: RefCell<HashMap<(SharedString, char), bool>>,
    /// Shaped and placed graphemes, for the font, size and scale in `shaped_for`.
    shapes: RefCell<HashMap<ShapeKey, Rc<[Placed]>>>,
    shaped_for: RefCell<Option<(SharedString, f32, f32)>>,
}

/// A grapheme, its bold and italic bits and the cells its box spans.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ShapeKey {
    Char(char, u8, u16),
    Str(Box<str>, u8, u16),
}

/// One glyph of a shaped grapheme, its pen position from the cell's top left.
#[derive(Clone, Copy, Debug)]
struct Placed {
    font: gpui::FontId,
    glyph: gpui::GlyphId,
    x: f32,
    y: f32,
    size: Pixels,
    emoji: bool,
}

impl NodeState {
    fn new(session: TerminalSession) -> Self {
        Self {
            session: RefCell::new(session),
            focus: RefCell::default(),
            focused: StdCell::new(false),
            waking: RefCell::default(),
            marked: RefCell::default(),
            pending_key: RefCell::default(),
            held: StdCell::default(),
            hover: RefCell::default(),
            sprites: RefCell::default(),
            layout: StdCell::default(),
            coverage: RefCell::default(),
            shapes: RefCell::default(),
            shaped_for: RefCell::default(),
        }
    }

    fn session(&self) -> TerminalSession {
        self.session.borrow().clone()
    }
}

/// The grid's metrics and where it sits, in logical pixels, with the device-pixel cell size.
#[derive(Clone, Copy, Debug, PartialEq)]
struct GridLayout {
    origin: Point<Pixels>,
    cell_width: Pixels,
    cell_height: Pixels,
    baseline: Pixels,
    /// From the baseline, down: where an underline's top sits.
    underline: Pixels,
    /// From the baseline, up: the middle of a strikethrough (half the face's x-height).
    strikethrough: Pixels,
    scale: f32,
    cols: u16,
    rows: u16,
}

impl GridLayout {
    /// Device pixels from the grid's top left, for the session's mouse input.
    fn local(&self, position: Point<Pixels>) -> (f32, f32) {
        ((position.x - self.origin.x).as_f32() * self.scale, (position.y - self.origin.y).as_f32() * self.scale)
    }

    fn cell_at(&self, position: Point<Pixels>) -> (u16, u16) {
        let x = ((position.x - self.origin.x) / self.cell_width).floor().max(0.) as u16;
        let y = ((position.y - self.origin.y) / self.cell_height).floor().max(0.) as u16;
        (x.min(self.cols.saturating_sub(1)), y.min(self.rows.saturating_sub(1)))
    }

    fn cell_origin(&self, col: u16, row: u16) -> Point<Pixels> {
        point(self.origin.x + self.cell_width * f32::from(col), self.origin.y + self.cell_height * f32::from(row))
    }
}

impl CustomWidget for GpuiTerminal {
    fn element(&self, cx: &CustomCx<'_>, window: &mut Window) -> AnyElement {
        let props = cx.props().downcast_ref::<TerminalProps>().expect("terminal node needs TerminalProps").clone();
        if !self.fonts_loaded.replace(true) {
            // For shaping only: GPUI skips faces without an `m`, which this one is, so icons
            // draw through `icons` instead; registering keeps a user's fallback lookups sane.
            let _ = window.text_system().add_fonts(vec![Cow::Borrowed(SYMBOLS_FONT)]);
        }
        // A node showing another session starts over: its caches hold the old one's state, and
        // in tests the old one's app (font ids belong to a text system).
        let state = {
            let mut nodes = self.nodes.borrow_mut();
            let fresh = nodes.get(&cx.node()).is_none_or(|s| *s.session.borrow() != props.session);
            if fresh {
                nodes.insert(cx.node(), Rc::new(NodeState::new(props.session.clone())));
            }
            nodes[&cx.node()].clone()
        };
        let theme_tokens = cx.theme();
        let theme = TerminalTheme::from_theme(&theme_tokens, &props.colors);
        let session = state.session();
        session.set_theme(&theme);
        let more = session.pump();
        let emit = cx.emitter();
        let emitted = emit_session_events(&session, &emit);
        let font_size = props.font_size.unwrap_or(DEFAULT_FONT_SIZE);
        record_paints(cx, &session, &theme, font_size);
        TerminalElement {
            node: cx.node(),
            state,
            props,
            theme,
            tokens: theme_tokens,
            emit,
            wake: cx.waker(),
            wake_now: emitted,
            more,
        }
        .into_any_element()
    }

    fn released(&self, node: NodeId) {
        self.nodes.borrow_mut().remove(&node);
    }
}

/// What the appearance audit reads.
fn record_paints(cx: &CustomCx<'_>, session: &TerminalSession, theme: &TerminalTheme, font_size: f32) {
    for mark in session.frame(|frame| paint::marks(frame, theme, font_size)) {
        match mark.text {
            Some((size, weight)) => cx.record_text(&mark.label, mark.color, &mark.under, size, weight),
            None => cx.record_fill(&mark.label, mark.color, &mark.under),
        }
    }
}

struct TerminalElement {
    node: NodeId,
    state: Rc<NodeState>,
    props: TerminalProps,
    theme: TerminalTheme,
    tokens: Rc<Theme>,
    emit: nd_ui::events::Emit,
    wake: Rc<dyn Fn(&mut App)>,
    wake_now: bool,
    more: bool,
}

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

/// What prepaint hands paint: the hitbox and the fonts.
struct Prepainted {
    hitbox: Hitbox,
    fonts: Fonts,
}

#[derive(Clone)]
struct Fonts {
    regular: Font,
    bold: Font,
    italic: Font,
    bold_italic: Font,
    size: Pixels,
}

impl Fonts {
    fn new(family: &str, size: f32) -> Self {
        let base = Font {
            family: SharedString::from(family.to_owned()),
            fallbacks: Some(FontFallbacks::from_fonts(vec![SYMBOLS_FAMILY.to_owned()])),
            ..gpui::font("")
        };
        Self {
            bold: Font { weight: FontWeight::BOLD, ..base.clone() },
            italic: Font { style: FontStyle::Italic, ..base.clone() },
            bold_italic: Font { weight: FontWeight::BOLD, style: FontStyle::Italic, ..base.clone() },
            regular: base,
            size: px(size),
        }
    }

    fn pick(&self, attrs: Attrs) -> &Font {
        match (attrs.contains(Attrs::BOLD), attrs.contains(Attrs::ITALIC)) {
            (false, false) => &self.regular,
            (true, false) => &self.bold,
            (false, true) => &self.italic,
            (true, true) => &self.bold_italic,
        }
    }
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepainted;

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::Integer(self.node.0))
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        gpui::Refineable::refine(&mut style, &self.props.style);
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepainted {
        let family = match &self.props.font_family {
            Some(f) if !f.is_empty() => f.to_string(),
            _ => self.tokens.fonts.mono.family.to_string(),
        };
        let fonts = Fonts::new(&family, self.props.font_size.unwrap_or(DEFAULT_FONT_SIZE));
        let layout = grid_layout(&fonts, bounds, px(self.props.padding), window);
        self.state.layout.set(Some(layout));
        let session = self.state.session();
        session.resize(GridSize {
            cols: layout.cols,
            rows: layout.rows,
            cell_width: (layout.cell_width.as_f32() * layout.scale).round() as u32,
            cell_height: (layout.cell_height.as_f32() * layout.scale).round() as u32,
        });
        self.ensure_waker(window, cx);
        if self.wake_now || emit_session_events(&session, &self.emit) {
            (self.wake)(cx);
        }
        if self.more || session.has_pending_output() {
            window.request_animation_frame();
        }
        let focus = self.state.focus.borrow_mut().get_or_insert_with(|| cx.focus_handle()).clone();
        window.set_focus_handle(&focus, cx);
        let focused = focus.is_focused(window);
        if self.state.focused.replace(focused) != focused {
            session.set_focused(focused);
            self.emit.emit(FocusChanged { focused });
            (self.wake)(cx);
        }
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        Prepainted { hitbox, fonts }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepainted: &mut Prepainted,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(layout) = self.state.layout.get() else { return };
        let focus = self.state.focus.borrow().clone().expect("prepaint made the focus handle");
        window.set_cursor_style(CursorStyle::IBeam, &prepainted.hitbox);
        self.listen(&prepainted.hitbox, &focus, window);
        window.handle_input(
            &focus,
            InputHandler { state: self.state.clone(), emit: self.emit.clone(), wake: self.wake.clone() },
            cx,
        );
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            window.paint_quad(fill(bounds, self.theme.background));
            let shaped_for = (prepainted.fonts.regular.family.clone(), prepainted.fonts.size.as_f32(), layout.scale);
            if self.state.shaped_for.borrow().as_ref() != Some(&shaped_for) {
                self.state.shapes.borrow_mut().clear();
                *self.state.shaped_for.borrow_mut() = Some(shaped_for);
            }
            let session = self.state.session();
            session.frame(|frame| self.paint_grid(frame, &layout, &prepainted.fonts, window, cx));
        });
    }
}

fn grid_layout(fonts: &Fonts, bounds: Bounds<Pixels>, padding: Pixels, window: &mut Window) -> GridLayout {
    let scale = window.scale_factor();
    let text = window.text_system();
    let id = text.resolve_font(&fonts.regular);
    // The widest printable ASCII advance, not the font's maximum: a Nerd Font's icons would
    // make that several cells wide.
    let width = (0x20u8..0x7f)
        .filter_map(|c| text.advance(id, fonts.size, char::from(c)).ok())
        .map(|a| a.width.as_f32())
        .fold(0f32, f32::max)
        .max(1.);
    let ascent = text.ascent(id, fonts.size).as_f32();
    let descent = text.descent(id, fonts.size).as_f32().abs();
    let x_height = text.x_height(id, fonts.size).as_f32();
    let snap = |v: f32| (v * scale).ceil() / scale;
    let cell_width = snap(width);
    let cell_height = snap(ascent + descent);
    let baseline = snap(ascent);
    let inner_w = (bounds.size.width.as_f32() - 2. * padding.as_f32()).max(0.);
    let inner_h = (bounds.size.height.as_f32() - 2. * padding.as_f32()).max(0.);
    let cols = (inner_w / cell_width).floor().max(1.) as u16;
    let rows = (inner_h / cell_height).floor().max(1.) as u16;
    let origin = point(
        px(((bounds.origin.x + padding).as_f32() * scale).round() / scale),
        px(((bounds.origin.y + padding).as_f32() * scale).round() / scale),
    );
    GridLayout {
        origin,
        cell_width: px(cell_width),
        cell_height: px(cell_height),
        baseline: px(baseline),
        // A third of the way into the descent, as text views place it, and clear of the
        // baseline by at least a device pixel.
        underline: px(snap(descent / 3.).max(1. / scale)),
        strikethrough: px(snap(x_height / 2.)),
        scale,
        cols,
        rows,
    }
}

impl TerminalElement {
    /// Redraws the window when the session has output, from a GPUI task fed by the I/O thread.
    fn ensure_waker(&self, window: &mut Window, cx: &mut App) {
        if self.state.waking.borrow().is_some() {
            return;
        }
        let (tx, mut rx) = mpsc::unbounded::<()>();
        let guard = self.state.session().on_output(move || {
            let _ = tx.unbounded_send(());
        });
        let handle = window.window_handle();
        let task = cx.spawn(async move |cx| {
            while rx.next().await.is_some() {
                // Several wakes before the next frame need one redraw.
                while rx.try_recv().is_ok() {}
                if handle.update(cx, |_, window, _| window.refresh()).is_err() {
                    break;
                }
            }
        });
        *self.state.waking.borrow_mut() = Some((guard, task));
    }

    fn listen(&self, hitbox: &Hitbox, focus: &FocusHandle, window: &mut Window) {
        let state = self.state.clone();
        let emit = self.emit.clone();
        let wake = self.wake.clone();
        let hit = hitbox.clone();
        let focus_down = focus.clone();
        window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !hit.is_hovered(window) {
                return;
            }
            window.focus(&focus_down, cx);
            let Some(layout) = state.layout.get() else { return };
            let button = mouse_button(e.button);
            let mods = mods_of(&e.modifiers);
            if button == Some(MouseButton::Left) && link_modifier(&e.modifiers) {
                let (col, row) = layout.cell_at(e.position);
                if let Some(link) = state.session().link_at(col, row) {
                    emit.emit(LinkActivated { link });
                    wake(cx);
                    return;
                }
            }
            if let Some(button) = button {
                state.held.set(Some(button));
                let (x, y) = layout.local(e.position);
                state.session().mouse_down(button, x, y, mods);
                after_input(&state, &emit, &wake, window, cx);
            }
        });
        let state = self.state.clone();
        let emit = self.emit.clone();
        let wake = self.wake.clone();
        window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let (Some(layout), Some(button)) = (state.layout.get(), mouse_button(e.button)) else { return };
            if state.held.get() != Some(button) {
                return;
            }
            state.held.set(None);
            let (x, y) = layout.local(e.position);
            state.session().mouse_up(button, x, y, mods_of(&e.modifiers));
            after_input(&state, &emit, &wake, window, cx);
        });
        let state = self.state.clone();
        let emit = self.emit.clone();
        let wake = self.wake.clone();
        let hit = hitbox.clone();
        window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let Some(layout) = state.layout.get() else { return };
            let held = state.held.get();
            if held.is_none() && !hit.is_hovered(window) {
                return;
            }
            let hover = if link_modifier(&e.modifiers) {
                let (col, row) = layout.cell_at(e.position);
                state.session().link_at(col, row)
            } else {
                None
            };
            if *state.hover.borrow() != hover {
                *state.hover.borrow_mut() = hover;
                window.refresh();
            }
            let (x, y) = layout.local(e.position);
            state.session().mouse_moved(x, y, held, mods_of(&e.modifiers));
            after_input(&state, &emit, &wake, window, cx);
        });
        let state = self.state.clone();
        let emit = self.emit.clone();
        let wake = self.wake.clone();
        let hit = hitbox.clone();
        window.on_mouse_event(move |e: &ScrollWheelEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !hit.is_hovered(window) {
                return;
            }
            let Some(layout) = state.layout.get() else { return };
            let dy = match e.delta {
                ScrollDelta::Pixels(p) => p.y.as_f32() * layout.scale,
                ScrollDelta::Lines(l) => l.y * layout.cell_height.as_f32() * layout.scale,
            };
            let (x, y) = layout.local(e.position);
            state.session().scroll_pixels(dy, x, y, mods_of(&e.modifiers));
            after_input(&state, &emit, &wake, window, cx);
            cx.stop_propagation();
        });
        let state = self.state.clone();
        let emit = self.emit.clone();
        let wake = self.wake.clone();
        let focus_key = focus.clone();
        window.on_key_event(move |e: &KeyDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !focus_key.is_focused(window) {
                return;
            }
            if key_down(&state, e, cx) {
                cx.stop_propagation();
                after_input(&state, &emit, &wake, window, cx);
            }
        });
        let state = self.state.clone();
        let focus_up = focus.clone();
        window.on_key_event(move |e: &KeyUpEvent, phase, window, _cx| {
            if phase != DispatchPhase::Bubble || !focus_up.is_focused(window) {
                return;
            }
            let key = key_for_name(&e.keystroke.key);
            let input =
                KeyInput { action: KeyAction::Release, ..KeyInput::press(key, mods_of(&e.keystroke.modifiers)) };
            state.session().key(&input);
        });
    }

    fn paint_grid(&self, frame: &Frame, layout: &GridLayout, fonts: &Fonts, window: &mut Window, cx: &mut App) {
        let theme = &self.theme;
        let (cw, ch) = (layout.cell_width, layout.cell_height);
        let cursor = frame.cursor;
        let block_cursor = cursor.filter(|c| c.shape == CursorShape::Block && self.state.marked.borrow().is_none());
        // Backgrounds, merged into runs of equal colour.
        for (y, row) in frame.rows.iter().enumerate() {
            let mut run: Option<(u16, u16, Hsla)> = None;
            for (x, cell) in row.cells.iter().enumerate() {
                let (_, bg) = cell_colors(cell, theme, frame);
                match (&mut run, bg) {
                    (Some((_, end, color)), Some(bg)) if *color == bg && *end == x as u16 => *end += 1,
                    (_, bg) => {
                        if let Some((start, end, color)) = run.take() {
                            let o = layout.cell_origin(start, y as u16);
                            window.paint_quad(fill(Bounds::new(o, size(cw * f32::from(end - start), ch)), color));
                        }
                        run = bg.map(|c| (x as u16, x as u16 + 1, c));
                    }
                }
            }
            if let Some((start, end, color)) = run {
                let o = layout.cell_origin(start, y as u16);
                window.paint_quad(fill(Bounds::new(o, size(cw * f32::from(end - start), ch)), color));
            }
        }
        let cursor_color = cursor.and_then(|c| c.color).map_or(theme.cursor, Rgb::to_hsla);
        if let Some(c) = block_cursor {
            let width = frame.cell(c.col, c.row).map_or(1, |(_, cell)| cell.width());
            window.paint_quad(fill(
                Bounds::new(layout.cell_origin(c.col, c.row), size(cw * f32::from(width), ch)),
                cursor_color,
            ));
        }
        // Text, one shaped line per row so GPUI's layout cache keeps it between frames.
        for (y, row) in frame.rows.iter().enumerate() {
            self.paint_row(frame, row, y as u16, layout, fonts, block_cursor, window);
        }
        // Cursor shapes drawn over the text.
        if let Some(c) = cursor.filter(|c| c.shape != CursorShape::Block || self.state.marked.borrow().is_some()) {
            let o = layout.cell_origin(c.col, c.row);
            let thick = px((1. * layout.scale).max(1.) / layout.scale);
            match c.shape {
                CursorShape::Bar => window.paint_quad(fill(Bounds::new(o, size(thick * 2., ch)), cursor_color)),
                CursorShape::Underline => window.paint_quad(fill(
                    Bounds::new(point(o.x, o.y + ch - thick * 2.), size(cw, thick * 2.)),
                    cursor_color,
                )),
                _ => window.paint_quad(gpui::outline(
                    Bounds::new(o, size(cw, ch)),
                    cursor_color,
                    gpui::BorderStyle::Solid,
                )),
            }
        }
        if let Some(link) = self.state.hover.borrow().as_ref() {
            let o = layout.cell_origin(link.start, link.row);
            let style = UnderlineStyle { thickness: px(1.), color: Some(theme.foreground), wavy: false };
            window.paint_underline(
                point(o.x, o.y + layout.baseline + px(2.)),
                cw * f32::from(link.end - link.start),
                &style,
            );
        }
        if let (Some(marked), Some(c)) = (self.state.marked.borrow().as_ref(), cursor) {
            self.paint_marked(marked, c.col, c.row, layout, fonts, window, cx);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_row(
        &self,
        frame: &Frame,
        row: &crate::frame::Row,
        y: u16,
        layout: &GridLayout,
        fonts: &Fonts,
        block_cursor: Option<crate::frame::Cursor>,
        window: &mut Window,
    ) {
        let theme = &self.theme;
        for (x, cell) in row.cells.iter().enumerate() {
            if cell.is_blank() || cell.attrs.contains(Attrs::INVISIBLE) {
                continue;
            }
            let g = row.grapheme(cell);
            let (mut fg, _) = cell_colors(cell, theme, frame);
            if block_cursor.is_some_and(|c| usize::from(c.col) == x && usize::from(c.row) == usize::from(y)) {
                fg = theme.background;
            }
            let o = layout.cell_origin(x as u16, y);
            let single = single_char(g);
            let cells = glyph::box_cells(row, x);
            if let Some(c) = single.filter(|c| sprite::is_sprite(*c)) {
                self.paint_mask(c, 1, o, layout, fg, window, |w, h| {
                    sprite::draw(c, w, h, (layout.scale.round() as u32).max(1))
                });
            } else if let Some(c) =
                single.filter(|c| glyph::is_symbol(*c) && icons::covers(*c) && !self.primary_covers(*c, fonts, window))
            {
                let ppem = fonts.size.as_f32() * layout.scale;
                let baseline = layout.baseline.as_f32() * layout.scale;
                self.paint_mask(c, cells, o, layout, fg, window, |w, h| icons::mask(c, ppem, w, h, baseline));
            } else {
                let placed = self.shaped(g, single, cell.attrs, cells, layout, fonts, window);
                for p in placed.iter() {
                    let origin = point(o.x + px(p.x), o.y + px(p.y));
                    if p.emoji {
                        let _ = window.paint_emoji(origin, p.font, p.glyph, p.size);
                    } else {
                        let _ = window.paint_glyph(origin, p.font, p.glyph, p.size, fg);
                    }
                }
            }
            self.paint_decorations(cell, o, layout, fg, window);
        }
    }

    /// The glyphs of grapheme `g` placed in its box: shaped once per grapheme, style and box
    /// size, so a frame of scrolled text costs lookups, not shaping.
    #[allow(clippy::too_many_arguments)]
    fn shaped(
        &self,
        g: &str,
        single: Option<char>,
        attrs: Attrs,
        cells: u16,
        layout: &GridLayout,
        fonts: &Fonts,
        window: &mut Window,
    ) -> Rc<[Placed]> {
        let style = u8::from(attrs.contains(Attrs::BOLD)) | u8::from(attrs.contains(Attrs::ITALIC)) << 1;
        let key = match single {
            Some(c) => ShapeKey::Char(c, style, cells),
            None => ShapeKey::Str(g.into(), style, cells),
        };
        if let Some(placed) = self.state.shapes.borrow().get(&key) {
            return placed.clone();
        }
        let (cw, ch) = (layout.cell_width, layout.cell_height);
        let run = TextRun {
            len: g.len(),
            font: fonts.pick(attrs).clone(),
            color: self.theme.foreground,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped = window.text_system().shape_line(SharedString::from(g.to_owned()), fonts.size, &[run], None);
        let symbol = single.is_some_and(glyph::is_symbol);
        let mut placed = Vec::new();
        for shaped_run in &shaped.runs {
            for (i, gl) in shaped_run.glyphs.iter().enumerate() {
                // The glyph's box in its own face: its advance by the face's ascent and descent.
                // Not ink: GPUI's typographic bounds are the glyph's ink on macOS but only its
                // advances on Linux, which read as a glyph a whole line tall above the baseline.
                let text = window.text_system();
                let next = shaped_run.glyphs.get(i + 1).map(|n| n.position.x);
                let advance = next.map_or(shaped.width, |x| x) - gl.position.x;
                let ascent = text.ascent(shaped_run.font_id, fonts.size).as_f32();
                let descent = text.descent(shaped_run.font_id, fonts.size).as_f32().abs();
                let ink = Ink { x: 0., y: -ascent, width: advance.as_f32(), height: ascent + descent };
                let fit = glyph::fit(
                    ink,
                    cw.as_f32() * f32::from(cells),
                    ch.as_f32(),
                    layout.baseline.as_f32(),
                    symbol || gl.is_emoji,
                );
                placed.push(Placed {
                    font: shaped_run.font_id,
                    glyph: gl.id,
                    x: fit.x + gl.position.x.as_f32() * fit.scale,
                    y: fit.y,
                    size: fonts.size * fit.scale,
                    emoji: gl.is_emoji,
                });
            }
        }
        let placed: Rc<[Placed]> = placed.into();
        let mut shapes = self.state.shapes.borrow_mut();
        if shapes.len() > 20_000 {
            shapes.clear();
        }
        shapes.insert(key, placed.clone());
        placed
    }

    /// True when the grid's own font draws `c`: a Nerd Font the user picked keeps its icons.
    fn primary_covers(&self, c: char, fonts: &Fonts, window: &mut Window) -> bool {
        let key = (fonts.regular.family.clone(), c);
        if let Some(known) = self.state.coverage.borrow().get(&key) {
            return *known;
        }
        let text = window.text_system();
        let id = text.resolve_font(&Font { fallbacks: None, ..fonts.regular.clone() });
        let covers = text.typographic_bounds(id, fonts.size, c).is_ok_and(|b| b.size.width.as_f32() > 0.);
        self.state.coverage.borrow_mut().insert(key, covers);
        covers
    }

    fn paint_decorations(
        &self,
        cell: &crate::frame::Cell,
        o: Point<Pixels>,
        layout: &GridLayout,
        fg: Hsla,
        window: &mut Window,
    ) {
        let cw = layout.cell_width * f32::from(cell.width());
        let thick = px((layout.scale).round().max(1.) / layout.scale);
        let color = cell.underline_color.map_or(fg, |c| self.theme.ink(c));
        let under_y = o.y + layout.baseline + layout.underline;
        match cell.underline {
            Underline::None => {}
            Underline::Single => window.paint_quad(fill(Bounds::new(point(o.x, under_y), size(cw, thick)), color)),
            Underline::Double => {
                window.paint_quad(fill(Bounds::new(point(o.x, under_y), size(cw, thick)), color));
                window.paint_quad(fill(Bounds::new(point(o.x, under_y + thick * 2.), size(cw, thick)), color));
            }
            Underline::Curly => {
                let style = UnderlineStyle { thickness: thick, color: Some(color), wavy: true };
                window.paint_underline(point(o.x, under_y), cw, &style);
            }
            Underline::Dotted | Underline::Dashed => {
                let dash = if cell.underline == Underline::Dotted { thick * 2. } else { cw / 3. };
                let mut x = o.x;
                while x < o.x + cw {
                    let w = dash.min(o.x + cw - x);
                    window.paint_quad(fill(Bounds::new(point(x, under_y), size(w, thick)), color));
                    x += dash * 2.;
                }
            }
        }
        if cell.attrs.contains(Attrs::STRIKETHROUGH) {
            let y = o.y + layout.baseline - layout.strikethrough - thick / 2.;
            window.paint_quad(fill(Bounds::new(point(o.x, y), size(cw, thick)), fg));
        }
        if cell.attrs.contains(Attrs::OVERLINE) {
            window.paint_quad(fill(Bounds::new(o, size(cw, thick)), fg));
        }
        if cell.attrs.contains(Attrs::LINK) && cell.underline == Underline::None {
            let faint = with_alpha(fg, fg.a * 0.4);
            window.paint_quad(fill(Bounds::new(point(o.x, under_y), size(cw, thick)), faint));
        }
    }

    /// Draws a coverage mask the size of `cells` cells in `color`: a sprite or an icon, cached
    /// as an image per glyph, size and colour.
    #[allow(clippy::too_many_arguments)]
    fn paint_mask(
        &self,
        c: char,
        cells: u16,
        o: Point<Pixels>,
        layout: &GridLayout,
        color: Hsla,
        window: &mut Window,
        draw: impl FnOnce(u32, u32) -> Option<sprite::Mask>,
    ) {
        let w = (layout.cell_width.as_f32() * f32::from(cells) * layout.scale).round() as u32;
        let h = (layout.cell_height.as_f32() * layout.scale).round() as u32;
        let rgba = color.to_rgb();
        let bytes = [rgba.r, rgba.g, rgba.b, rgba.a].map(|v| (v.clamp(0., 1.) * 255.).round() as u8);
        let key = (c, w, h, bytes);
        let image = {
            let mut sprites = self.state.sprites.borrow_mut();
            if sprites.len() > 4096 {
                sprites.clear();
            }
            match sprites.entry(key) {
                std::collections::hash_map::Entry::Occupied(e) => e.get().clone(),
                std::collections::hash_map::Entry::Vacant(e) => {
                    let Some(mask) = draw(w, h) else { return };
                    // GPUI images are BGRA.
                    let mut data = Vec::with_capacity(mask.alpha.len() * 4);
                    for a in &mask.alpha {
                        let alpha = (u16::from(*a) * u16::from(bytes[3]) / 255) as u8;
                        data.extend_from_slice(&[bytes[2], bytes[1], bytes[0], alpha]);
                    }
                    let buffer = image::RgbaImage::from_raw(w, h, data).expect("sized above");
                    e.insert(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))).clone()
                }
            }
        };
        let bounds = Bounds::new(o, size(layout.cell_width * f32::from(cells), layout.cell_height));
        let _ = window.paint_image(bounds, bounds, Default::default(), image, 0, false);
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_marked(
        &self,
        marked: &str,
        col: u16,
        row: u16,
        layout: &GridLayout,
        fonts: &Fonts,
        window: &mut Window,
        cx: &mut App,
    ) {
        let o = layout.cell_origin(col, row);
        let runs = [TextRun {
            len: marked.len(),
            font: fonts.regular.clone(),
            color: self.theme.foreground,
            background_color: None,
            underline: Some(UnderlineStyle { thickness: px(1.), color: Some(self.theme.foreground), wavy: false }),
            strikethrough: None,
        }];
        let shaped = window.text_system().shape_line(SharedString::from(marked.to_owned()), fonts.size, &runs, None);
        window.paint_quad(fill(Bounds::new(o, size(shaped.width, layout.cell_height)), self.theme.background));
        let _ = shaped.paint(o, layout.cell_height, gpui::TextAlign::Left, None, window, cx);
    }
}

fn single_char(g: &str) -> Option<char> {
    let mut chars = g.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

fn mouse_button(b: gpui::MouseButton) -> Option<MouseButton> {
    match b {
        gpui::MouseButton::Left => Some(MouseButton::Left),
        gpui::MouseButton::Right => Some(MouseButton::Right),
        gpui::MouseButton::Middle => Some(MouseButton::Middle),
        _ => None,
    }
}

pub(crate) fn mods_of(m: &Modifiers) -> Mods {
    let mut mods = Mods::empty();
    if m.shift {
        mods |= Mods::SHIFT;
    }
    if m.control {
        mods |= Mods::CTRL;
    }
    if m.alt {
        mods |= Mods::ALT;
    }
    if m.platform {
        mods |= Mods::SUPER;
    }
    mods
}

/// Cmd on macOS, Ctrl elsewhere, as in every terminal's open-link gesture.
fn link_modifier(m: &Modifiers) -> bool {
    if cfg!(target_os = "macos") { m.platform } else { m.control }
}

/// Copy, paste and select all on the platform's shortcuts: Cmd-C/V/A on macOS, Ctrl-Shift-C/V/A
/// elsewhere (plain Ctrl-C is the program's interrupt).
fn clipboard_shortcut(e: &KeyDownEvent) -> Option<&'static str> {
    let m = &e.keystroke.modifiers;
    let chord = if cfg!(target_os = "macos") {
        m.platform && !m.control && !m.alt
    } else {
        m.control && m.shift && !m.alt && !m.platform
    };
    if !chord {
        return None;
    }
    match e.keystroke.key.to_ascii_lowercase().as_str() {
        "c" => Some("copy"),
        "v" => Some("paste"),
        "a" => Some("select-all"),
        _ => None,
    }
}

/// A key-down for the program. Printable text without Ctrl or Cmd is left to the input handler,
/// which an input method may compose first. True when the key was taken.
fn key_down(state: &NodeState, e: &KeyDownEvent, cx: &mut App) -> bool {
    let session = state.session();
    match clipboard_shortcut(e) {
        Some("copy") => {
            if let Some(text) = session.selection_text() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            return true;
        }
        Some("paste") => {
            if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                session.paste(&text);
            }
            return true;
        }
        Some(_) => {
            session.select_all();
            return true;
        }
        None => {}
    }
    if state.marked.borrow().is_some() {
        return false;
    }
    let k = &e.keystroke;
    let key = key_for_name(&k.key);
    let mods = mods_of(&k.modifiers);
    *state.pending_key.borrow_mut() = Some((key, mods));
    let typed = k.key_char.as_deref().filter(|t| !t.is_empty() && !t.chars().any(char::is_control));
    if typed.is_some() && !k.modifiers.control && !k.modifiers.platform {
        // The input handler delivers this text (and anything an input method composes).
        return false;
    }
    let action = if e.is_held { KeyAction::Repeat } else { KeyAction::Press };
    let mut input = KeyInput { action, ..KeyInput::press(key, mods) };
    if key == Key::Unidentified {
        input.unshifted = k.key.chars().next().unwrap_or('\0');
    } else {
        input.unshifted = unshifted_us(key);
    }
    session.key(&input)
}

fn after_input(
    state: &NodeState,
    emit: &nd_ui::events::Emit,
    wake: &Rc<dyn Fn(&mut App)>,
    window: &mut Window,
    cx: &mut App,
) {
    if emit_session_events(&state.session(), emit) {
        wake(cx);
    }
    window.refresh();
}

/// Text input for the focused terminal: committed text goes to the program, composing text is
/// drawn at the cursor.
struct InputHandler {
    state: Rc<NodeState>,
    emit: nd_ui::events::Emit,
    wake: Rc<dyn Fn(&mut App)>,
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

impl gpui::InputHandler for InputHandler {
    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut App) -> Option<UTF16Selection> {
        let len = self.state.marked.borrow().as_deref().map_or(0, utf16_len);
        Some(UTF16Selection { range: len..len, reversed: false })
    }

    fn marked_text_range(&mut self, _: &mut Window, _: &mut App) -> Option<Range<usize>> {
        self.state.marked.borrow().as_deref().map(|m| 0..utf16_len(m))
    }

    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<String> {
        let marked = self.state.marked.borrow().clone()?;
        let units: Vec<u16> = marked.encode_utf16().collect();
        let end = range.end.min(units.len());
        let start = range.start.min(end);
        *adjusted = Some(start..end);
        String::from_utf16(&units[start..end]).ok()
    }

    fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, window: &mut Window, cx: &mut App) {
        self.state.marked.borrow_mut().take();
        let session = self.state.session();
        let pending = self.state.pending_key.borrow_mut().take();
        let single = single_char(text).is_some();
        match pending {
            // The text of the key just pressed: encode it as that key, so the Kitty protocol can
            // report it.
            Some((key, mods)) if single && key != Key::Unidentified => {
                session.key(&KeyInput::typed(key, mods, text));
            }
            _ => session.commit_text(text),
        }
        after_input(&self.state, &self.emit, &self.wake, window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        window: &mut Window,
        _: &mut App,
    ) {
        *self.state.marked.borrow_mut() = (!text.is_empty()).then(|| text.to_owned());
        window.refresh();
    }

    fn unmark_text(&mut self, window: &mut Window, _: &mut App) {
        self.state.marked.borrow_mut().take();
        window.refresh();
    }

    fn bounds_for_range(&mut self, _: Range<usize>, _: &mut Window, _: &mut App) -> Option<Bounds<Pixels>> {
        // The input method's candidate window goes under the cursor.
        let layout = self.state.layout.get()?;
        let cursor = self.state.session().frame(|f| f.cursor)?;
        Some(Bounds::new(layout.cell_origin(cursor.col, cursor.row), size(layout.cell_width, layout.cell_height)))
    }

    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut App) -> Option<usize> {
        None
    }

    fn apple_press_and_hold_enabled(&mut self) -> bool {
        false
    }
}
