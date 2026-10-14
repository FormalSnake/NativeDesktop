//! A terminal session: libghostty-vt's terminal, the transport feeding it, and the input
//! encoders, on the UI thread. The app owns it (a cheap handle) and hands it to a `terminal()`
//! element, so it outlives renders, moves between windows and can be driven without a window.

use std::cell::{Cell as StdCell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::io::{self, Read, Write};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use libghostty_vt::fmt::Format;
use libghostty_vt::key::{self, KittyKeyFlags};
use libghostty_vt::mouse;
use libghostty_vt::paste;
use libghostty_vt::render::{CellIterator, CursorVisualStyle, Dirty, RenderState, RowId, RowIterator};
use libghostty_vt::screen::CellWide;
use libghostty_vt::selection::FormatOptions;
use libghostty_vt::selection::gesture::{DragEvent, Geometry, Gesture, PressEvent, ReleaseEvent};
use libghostty_vt::style::{Palette, PaletteIndex, StyleColor};
use libghostty_vt::terminal::{
    ClipboardMime, ColorScheme, ConformanceLevel, DeviceAttributeFeature, DeviceAttributes, DeviceType, Mode, Point,
    PointCoordinate, PrimaryDeviceAttributes, ScrollViewport, SecondaryDeviceAttributes, SizeReportSize,
};
use libghostty_vt::{Error as VtError, Terminal};

use crate::frame::{Attrs, Cell, Cursor, CursorShape, Frame, Row, Underline};
use crate::input::{KeyInput, Mods, MouseButton};
use crate::io::{Command, ExitStatus, Transport, Waker};
use crate::links::{self, Link};
use crate::theme::TerminalTheme;

/// Lines kept above the screen unless the app asks for another number.
pub const DEFAULT_SCROLLBACK: usize = 10_000;
/// Output parsed per pump; more waits for the next frame so input and drawing keep up.
const PUMP_BUDGET: usize = 4 << 20;

/// Something the session reports to the app.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionEvent {
    TitleChanged(String),
    /// The working directory a program reported (OSC 7), as a path when it was a `file://` URL.
    CwdChanged(String),
    Bell,
    Exited(ExitStatus),
    SelectionChanged(bool),
    /// OSC 9 or OSC 777.
    Notification {
        title: String,
        body: String,
    },
}

/// Options a session starts with.
#[derive(Clone, Debug)]
pub struct SessionOptions {
    pub cols: u16,
    pub rows: u16,
    pub scrollback: usize,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self { cols: 80, rows: 24, scrollback: DEFAULT_SCROLLBACK }
    }
}

/// Size of the grid and of one cell in device pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u32,
    pub cell_height: u32,
}

/// Where the session's answers to the program go.
enum Sink {
    Transport(std::sync::mpsc::Sender<Vec<u8>>),
    Captured(Vec<u8>),
}

#[derive(Default)]
struct Effects {
    events: VecDeque<SessionEvent>,
    out: Vec<u8>,
}

/// A handle to one terminal session. Clones share the session; equality is identity.
#[derive(Clone)]
pub struct TerminalSession(Rc<Inner>);

impl PartialEq for TerminalSession {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for TerminalSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.0.state.try_borrow();
        let mut d = f.debug_struct("TerminalSession");
        if let Ok(s) = state {
            d.field("cols", &s.size.cols).field("rows", &s.size.rows).field("exited", &s.exit);
        }
        d.finish_non_exhaustive()
    }
}

struct Inner {
    state: RefCell<State>,
    effects: Rc<RefCell<Effects>>,
    size_report: Rc<StdCell<SizeReportSize>>,
    scheme: Rc<StdCell<Option<ColorScheme>>>,
    transport: Option<Transport>,
    next_waker: StdCell<u64>,
}

struct State {
    term: Terminal<'static, 'static>,
    render: RenderState<'static>,
    row_iter: RowIterator<'static>,
    cell_iter: CellIterator<'static>,
    key_encoder: key::Encoder<'static>,
    key_event: key::Event<'static>,
    mouse_encoder: mouse::Encoder<'static>,
    mouse_event: mouse::Event<'static>,
    gesture: Gesture<'static>,
    press: PressEvent<'static>,
    release: ReleaseEvent<'static>,
    drag: DragEvent<'static>,
    epoch: Instant,
    sink: Sink,
    size: GridSize,
    frame: Frame,
    row_ids: Vec<RowId>,
    /// Set when something outside libghostty's dirty tracking changed what draws.
    force_full: bool,
    theme: Option<TerminalTheme>,
    focused: bool,
    exit: Option<ExitStatus>,
    closed: bool,
    had_selection: bool,
    /// A left press that may become a selection drag.
    selecting: bool,
    /// The button the program got a press for, so the release and drags go to it too.
    reported_button: Option<MouseButton>,
    wheel_rest: f32,
    chunks: Vec<Vec<u8>>,
    grapheme: String,
}

impl TerminalSession {
    /// Runs `command` on a new pseudo-terminal.
    pub fn spawn(command: &Command, options: SessionOptions) -> io::Result<Self> {
        let transport = Transport::spawn(command, options.cols, options.rows)?;
        Self::with_transport(Some(transport), options).map_err(io::Error::other)
    }

    /// A session over any byte stream, such as a socket to a remote shell.
    pub fn connect(
        reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
        options: SessionOptions,
    ) -> io::Result<Self> {
        let transport = Transport::stream(reader, writer)?;
        Self::with_transport(Some(transport), options).map_err(io::Error::other)
    }

    /// A session with no transport: output arrives through [`Self::feed`] and what the
    /// terminal answers collects for [`Self::take_output`].
    pub fn detached(options: SessionOptions) -> Self {
        Self::with_transport(None, options).expect("a detached terminal needs no I/O")
    }

    fn with_transport(transport: Option<Transport>, options: SessionOptions) -> Result<Self, VtError> {
        let effects: Rc<RefCell<Effects>> = Rc::default();
        let size_report = Rc::new(StdCell::new(SizeReportSize {
            rows: options.rows,
            columns: options.cols,
            cell_width: 1,
            cell_height: 1,
        }));
        let scheme: Rc<StdCell<Option<ColorScheme>>> = Rc::default();
        let mut term = Terminal::new(options.cols, options.rows)?;
        // The line count is the limit apps ask for; libghostty's default byte cap would cut a
        // wide grid's history well short of it.
        term.set_scrollback_max_lines(Some(options.scrollback))?
            .set_scrollback_max_bytes(None)?
            .set_terminfo_name("xterm-256color")?;
        {
            let e = effects.clone();
            term.on_pty_write(move |_, data| e.borrow_mut().out.extend_from_slice(data))?;
            let e = effects.clone();
            term.on_bell(move |_| e.borrow_mut().events.push_back(SessionEvent::Bell))?;
            let e = effects.clone();
            term.on_title_changed(move |t| {
                let title = t.title().unwrap_or_default().to_owned();
                e.borrow_mut().events.push_back(SessionEvent::TitleChanged(title));
            })?;
            let e = effects.clone();
            term.on_pwd_changed(move |t| {
                let pwd = t.pwd().unwrap_or_default();
                e.borrow_mut().events.push_back(SessionEvent::CwdChanged(cwd_path(pwd)));
            })?;
            let e = effects.clone();
            term.on_desktop_notification(move |_, n| {
                let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
                let event = SessionEvent::Notification { title: text(n.title()), body: text(n.body()) };
                e.borrow_mut().events.push_back(event);
            })?;
            let s = size_report.clone();
            term.on_size(move |_| Some(s.get()))?;
            let c = scheme.clone();
            term.on_color_scheme(move |_| c.get())?;
            term.on_device_attributes(|_| {
                Some(DeviceAttributes {
                    primary: PrimaryDeviceAttributes::new(
                        ConformanceLevel::VT220,
                        &[
                            DeviceAttributeFeature::COLUMNS_132,
                            DeviceAttributeFeature::SELECTIVE_ERASE,
                            DeviceAttributeFeature::ANSI_COLOR,
                        ],
                    ),
                    secondary: SecondaryDeviceAttributes {
                        device_type: DeviceType::VT220,
                        firmware_version: 1,
                        rom_cartridge: 0,
                    },
                    tertiary: Default::default(),
                })
            })?;
            term.on_xtversion(|_| Some("nativedesktop"))?;
        }
        let sink = match &transport {
            Some(t) => Sink::Transport(t.input()),
            None => Sink::Captured(Vec::new()),
        };
        let mut press = PressEvent::new()?;
        press.set_repeat_interval(Duration::from_millis(500))?;
        let state = State {
            term,
            render: RenderState::new()?,
            row_iter: RowIterator::new()?,
            cell_iter: CellIterator::new()?,
            key_encoder: key::Encoder::new()?,
            key_event: key::Event::new()?,
            mouse_encoder: mouse::Encoder::new()?,
            mouse_event: mouse::Event::new()?,
            gesture: Gesture::new()?,
            press,
            release: ReleaseEvent::new()?,
            drag: DragEvent::new()?,
            epoch: Instant::now(),
            sink,
            size: GridSize { cols: options.cols, rows: options.rows, cell_width: 1, cell_height: 1 },
            frame: Frame::default(),
            row_ids: Vec::new(),
            force_full: true,
            theme: None,
            focused: false,
            exit: None,
            closed: false,
            had_selection: false,
            selecting: false,
            reported_button: None,
            wheel_rest: 0.,
            chunks: Vec::new(),
            grapheme: String::new(),
        };
        Ok(Self(Rc::new(Inner {
            state: RefCell::new(state),
            effects,
            size_report,
            scheme,
            transport,
            next_waker: StdCell::new(1),
        })))
    }

    /// Calls `waker` from an I/O thread when output or an exit is waiting for [`Self::pump`].
    /// The returned guard unregisters it.
    pub fn on_output(&self, waker: impl Fn() + Send + Sync + 'static) -> WakerGuard {
        let id = self.0.next_waker.get();
        self.0.next_waker.set(id + 1);
        if let Some(t) = &self.0.transport {
            t.shared.add_waker(id, Arc::new(waker) as Waker);
        }
        WakerGuard { session: Rc::downgrade(&self.0), id }
    }

    /// Parses output the transport delivered, up to a budget. True when more is waiting, so
    /// the caller should pump again on its next frame.
    pub fn pump(&self) -> bool {
        let Some(transport) = &self.0.transport else { return false };
        let mut state = self.0.state.borrow_mut();
        let mut chunks = std::mem::take(&mut state.chunks);
        let drained = transport.shared.drain(PUMP_BUDGET, &mut chunks);
        for chunk in chunks.drain(..) {
            state.term.vt_write(&chunk);
        }
        state.chunks = chunks;
        if drained.closed {
            state.closed = true;
        }
        let exited = drained.exit.filter(|_| state.exit.is_none());
        if let Some(exit) = &exited {
            state.exit = Some(exit.clone());
        }
        drop(state);
        if let Some(exit) = exited {
            self.0.effects.borrow_mut().events.push_back(SessionEvent::Exited(exit));
        }
        self.flush();
        drained.more
    }

    /// True while output is waiting for [`Self::pump`].
    pub fn has_pending_output(&self) -> bool {
        self.0.transport.as_ref().is_some_and(|t| t.shared.pending())
    }

    /// Feeds program output straight into the terminal, as if the transport delivered it.
    pub fn feed(&self, bytes: &[u8]) {
        self.0.state.borrow_mut().term.vt_write(bytes);
        self.flush();
    }

    /// Writes bytes to the program, as typed input. Scrolls to the bottom, as typing does.
    pub fn write(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let mut state = self.0.state.borrow_mut();
        state.term.scroll_viewport(ScrollViewport::Bottom);
        state.send(bytes.to_vec());
    }

    /// What the terminal answered a detached session's program with, since the last call.
    pub fn take_output(&self) -> Vec<u8> {
        self.flush();
        match &mut self.0.state.borrow_mut().sink {
            Sink::Captured(out) => std::mem::take(out),
            Sink::Transport(_) => Vec::new(),
        }
    }

    /// Events since the last call, oldest first.
    pub fn take_events(&self) -> Vec<SessionEvent> {
        self.flush();
        self.0.effects.borrow_mut().events.drain(..).collect()
    }

    /// Sends what the terminal's callbacks queued for the program, and reports a selection
    /// change.
    fn flush(&self) {
        let out = std::mem::take(&mut self.0.effects.borrow_mut().out);
        let mut state = self.0.state.borrow_mut();
        if !out.is_empty() {
            state.send(out);
        }
        let has = state.term.selection().ok().flatten().is_some();
        if has != state.had_selection {
            state.had_selection = has;
            drop(state);
            self.0.effects.borrow_mut().events.push_back(SessionEvent::SelectionChanged(has));
        }
    }

    pub fn size(&self) -> GridSize {
        self.0.state.borrow().size
    }

    /// Resizes the grid (rewrapping its text) and tells the program. Cell sizes are device
    /// pixels, for programs that ask for the window size.
    pub fn resize(&self, size: GridSize) {
        let size = GridSize { cols: size.cols.max(1), rows: size.rows.max(1), ..size };
        let mut state = self.0.state.borrow_mut();
        if state.size == size {
            return;
        }
        let grid_changed = (state.size.cols, state.size.rows) != (size.cols, size.rows);
        state.size = size;
        self.0.size_report.set(SizeReportSize {
            rows: size.rows,
            columns: size.cols,
            cell_width: size.cell_width,
            cell_height: size.cell_height,
        });
        let _ = state.term.resize(size.cols, size.rows, size.cell_width, size.cell_height);
        state.force_full = true;
        drop(state);
        if grid_changed && let Some(t) = &self.0.transport {
            t.resize(
                size.cols,
                size.rows,
                u32::from(size.cols) * size.cell_width,
                u32::from(size.rows) * size.cell_height,
            );
        }
        self.flush();
    }

    /// Applies a theme: default colours, the 16 ANSI colours, and the scheme reported to
    /// programs that ask (CSI ? 996 n).
    pub fn set_theme(&self, theme: &TerminalTheme) {
        let mut state = self.0.state.borrow_mut();
        if state.theme.as_ref() == Some(theme) {
            return;
        }
        let mut palette: Palette =
            state.term.default_color_palette().unwrap_or_else(|_| Palette([Default::default(); 256]));
        for (i, c) in theme.ansi_rgb.iter().enumerate() {
            palette.set(PaletteIndex(i as u8), (*c).into());
        }
        let _ = state
            .term
            .set_default_fg_color(Some(theme.foreground_rgb.into()))
            .and_then(|t| t.set_default_bg_color(Some(theme.background_rgb.into())))
            .and_then(|t| t.set_default_color_palette(Some(palette)));
        self.0.scheme.set(Some(if theme.dark { ColorScheme::Dark } else { ColorScheme::Light }));
        state.theme = Some(theme.clone());
        state.force_full = true;
    }

    pub fn theme(&self) -> Option<TerminalTheme> {
        self.0.state.borrow().theme.clone()
    }

    pub fn title(&self) -> String {
        self.0.state.borrow().term.title().unwrap_or_default().to_owned()
    }

    pub fn cwd(&self) -> String {
        cwd_path(self.0.state.borrow().term.pwd().unwrap_or_default())
    }

    pub fn exit_status(&self) -> Option<ExitStatus> {
        self.0.state.borrow().exit.clone()
    }

    /// True once the program closed its output (it exited, or the stream ended).
    pub fn is_closed(&self) -> bool {
        self.0.state.borrow().closed
    }

    /// The child's process id, for a session on a pseudo-terminal.
    pub fn pid(&self) -> Option<u32> {
        self.0.transport.as_ref().and_then(|t| t.pid)
    }

    /// Updates the viewport snapshot from the terminal and runs `f` on it. Rows libghostty did
    /// not mark dirty keep their cached cells.
    pub fn frame<R>(&self, f: impl FnOnce(&Frame) -> R) -> R {
        let mut state = self.0.state.borrow_mut();
        state.update_frame();
        f(&state.frame)
    }

    /// The viewport's text, one line per row, trailing blanks trimmed.
    pub fn screen_text(&self) -> String {
        self.frame(Frame::text)
    }

    /// Lines in the scrollback above the screen.
    pub fn scrollback_rows(&self) -> usize {
        self.0.state.borrow().term.scrollback_rows().unwrap_or(0)
    }

    /// Scrolls the viewport by whole rows; negative goes back into the scrollback.
    pub fn scroll_rows(&self, rows: isize) {
        let mut state = self.0.state.borrow_mut();
        state.term.scroll_viewport(ScrollViewport::Delta(rows));
    }

    pub fn scroll_to_top(&self) {
        self.0.state.borrow_mut().term.scroll_viewport(ScrollViewport::Top);
    }

    pub fn scroll_to_bottom(&self) {
        self.0.state.borrow_mut().term.scroll_viewport(ScrollViewport::Bottom);
    }

    /// A wheel or trackpad scroll of `dy` device pixels (positive scrolls content down, back in
    /// history, as `NSEvent.scrollingDeltaY` does). Programs that track the mouse get wheel
    /// buttons; the alternate screen with alternate scroll on gets arrow keys; otherwise the
    /// viewport moves through the scrollback.
    pub fn scroll_pixels(&self, dy: f32, x: f32, y: f32, mods: Mods) {
        let mut state = self.0.state.borrow_mut();
        let cell = state.size.cell_height.max(1) as f32;
        let total = state.wheel_rest + dy;
        let rows = (total / cell).trunc();
        state.wheel_rest = total - rows * cell;
        let rows = rows as isize;
        if rows == 0 {
            return;
        }
        if state.term.is_mouse_tracking().unwrap_or(false) {
            let button = if rows > 0 { mouse::Button::Four } else { mouse::Button::Five };
            for _ in 0..rows.unsigned_abs().min(8) {
                state.encode_mouse(mouse::Action::Press, Some(button), x, y, mods);
            }
            return;
        }
        let alternate = state.term.active_screen().is_ok_and(|s| s == libghostty_vt::screen::Screen::Alternate);
        if alternate && state.term.mode(Mode::ALT_SCROLL).unwrap_or(false) {
            let key = if rows > 0 { "\x1b[A" } else { "\x1b[B" };
            let app = state.term.mode(Mode::DECCKM).unwrap_or(false);
            let key = if app { key.replace('[', "O") } else { key.to_owned() };
            let bytes = key.repeat(rows.unsigned_abs());
            state.send(bytes.into_bytes());
            return;
        }
        state.term.scroll_viewport(ScrollViewport::Delta(-rows));
    }

    /// A key event. True when it produced input for the program.
    pub fn key(&self, input: &KeyInput) -> bool {
        let mut state = self.0.state.borrow_mut();
        let state = &mut *state;
        state
            .key_event
            .set_action(input.action)
            .set_key(input.key)
            .set_mods(input.mods)
            .set_consumed_mods(input.consumed)
            .set_unshifted_codepoint(input.unshifted)
            .set_composing(input.composing)
            .set_utf8(input.text.clone());
        let mut out = Vec::new();
        let ok = state.key_encoder.set_options_from_terminal(&state.term).encode_to_vec(&state.key_event, &mut out);
        if ok.is_err() || out.is_empty() {
            return false;
        }
        state.term.scroll_viewport(ScrollViewport::Bottom);
        state.send(out);
        true
    }

    /// Text an input method committed, written as typed.
    pub fn commit_text(&self, text: &str) {
        self.write(text.as_bytes());
    }

    /// Kitty keyboard flags the program asked for (0 when it did not opt in).
    pub fn kitty_flags(&self) -> KittyKeyFlags {
        self.0.state.borrow().term.kitty_keyboard_flags().unwrap_or(KittyKeyFlags::empty())
    }

    /// Pastes text: bracketed when the program asked for it, control bytes made safe.
    pub fn paste(&self, text: &str) {
        let mut state = self.0.state.borrow_mut();
        state.term.scroll_viewport(ScrollViewport::Bottom);
        let mimes = [ClipboardMime::new("text/plain")];
        let read = |_mime: &str, out: &mut dyn Write| out.write_all(text.as_bytes());
        if let Err(VtError::Rejected) = state.term.paste(paste::Options::new(), &mimes, read) {
            // The user pasted on purpose; the unsafe rule exists for programs that inject. The
            // unbracketed text still has its newlines turned into returns.
            let _ = state.term.paste(paste::Options::new().with_allow_unsafe(true), &mimes, read);
        }
        drop(state);
        self.flush();
    }

    /// Focus moved to or from the terminal: the cursor hollows, and a program that asked for
    /// focus reports (mode 1004) gets one.
    pub fn set_focused(&self, focused: bool) {
        let mut state = self.0.state.borrow_mut();
        if state.focused == focused {
            return;
        }
        state.focused = focused;
        state.force_full = true;
        if state.term.mode(Mode::FOCUS_EVENT).unwrap_or(false) {
            let event = if focused { libghostty_vt::focus::Event::Gained } else { libghostty_vt::focus::Event::Lost };
            let mut buf = [0u8; 8];
            if let Ok(n) = event.encode(&mut buf) {
                state.send(buf[..n].to_vec());
            }
        }
    }

    pub fn is_focused(&self) -> bool {
        self.0.state.borrow().focused
    }

    /// A press at `(x, y)`, device pixels from the grid's top left. A left press starts a
    /// selection unless the program tracks the mouse (Shift overrides that).
    pub fn mouse_down(&self, button: MouseButton, x: f32, y: f32, mods: Mods) {
        let mut state = self.0.state.borrow_mut();
        let tracking = state.term.is_mouse_tracking().unwrap_or(false);
        if button == MouseButton::Left && (!tracking || mods.contains(Mods::SHIFT)) {
            state.selecting = true;
            let point = state.point_at(x, y);
            let elapsed = state.epoch.elapsed();
            let distance = f64::from(state.size.cell_width);
            let State { term, gesture, press, .. } = &mut *state;
            if let Ok(grid) = term.grid_ref(point)
                && let Ok(p) = press.set_repeat_distance(distance)
                && let Ok(p) = p.set_time(elapsed)
                && let Ok(p) = p.set_position(f64::from(x), f64::from(y))
                && let Ok(selection) = p.apply(gesture, term, grid)
            {
                let _ = term.set_selection(selection.as_ref());
            }
            drop(state);
            self.flush();
            return;
        }
        if tracking {
            state.reported_button = Some(button);
            state.encode_mouse(mouse::Action::Press, Some(button.into()), x, y, mods);
        }
    }

    pub fn mouse_up(&self, button: MouseButton, x: f32, y: f32, mods: Mods) {
        let mut state = self.0.state.borrow_mut();
        if button == MouseButton::Left && state.selecting {
            state.selecting = false;
            let point = state.point_at(x, y);
            let State { term, gesture, release, .. } = &mut *state;
            let grid = term.grid_ref(point).ok();
            let _ = release.apply(gesture, term, grid);
            drop(state);
            self.flush();
            return;
        }
        if state.reported_button == Some(button) {
            state.reported_button = None;
            state.encode_mouse(mouse::Action::Release, Some(button.into()), x, y, mods);
        }
    }

    /// Pointer motion, with the button held if any.
    pub fn mouse_moved(&self, x: f32, y: f32, held: Option<MouseButton>, mods: Mods) {
        let mut state = self.0.state.borrow_mut();
        if state.selecting && held == Some(MouseButton::Left) {
            let point = state.point_at(x, y);
            let geometry = Geometry {
                columns: u32::from(state.size.cols),
                cell_width: state.size.cell_width,
                padding_left: 0,
                screen_height: u32::from(state.size.rows) * state.size.cell_height,
            };
            let rectangle = mods.contains(Mods::ALT);
            let State { term, gesture, drag, .. } = &mut *state;
            if let Ok(grid) = term.grid_ref(point)
                && let Ok(d) = drag.set_rectangle(rectangle)
                && let Ok(d) = d.set_position(f64::from(x), f64::from(y))
                && let Ok(selection) = d.apply(gesture, term, grid, geometry)
            {
                let _ = term.set_selection(selection.as_ref());
            }
            drop(state);
            self.flush();
            return;
        }
        if state.term.is_mouse_tracking().unwrap_or(false) {
            let button = held.or(state.reported_button).map(Into::into);
            state.encode_mouse(mouse::Action::Motion, button, x, y, mods);
        }
    }

    /// The selection as plain text, soft wraps joined and trailing blanks trimmed.
    pub fn selection_text(&self) -> Option<String> {
        let state = self.0.state.borrow();
        let options = FormatOptions::new().with_emit_format(Format::Plain).with_unwrap(true).with_trim(true);
        let bytes = state.term.format_selection_alloc(None, options).ok().flatten()?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }

    pub fn has_selection(&self) -> bool {
        self.0.state.borrow().term.selection().ok().flatten().is_some()
    }

    pub fn select_all(&self) {
        {
            let state = self.0.state.borrow();
            if let Ok(selection) = state.term.select_all() {
                let _ = state.term.set_selection(selection.as_ref());
            }
        }
        self.flush();
    }

    pub fn clear_selection(&self) {
        {
            let state = self.0.state.borrow();
            let _ = state.term.set_selection(None);
        }
        self.flush();
    }

    /// The link under a cell of the viewport: an OSC 8 hyperlink, or a URL in the row's text.
    pub fn link_at(&self, col: u16, row: u16) -> Option<Link> {
        let mut state = self.0.state.borrow_mut();
        let point = Point::Viewport(PointCoordinate { x: col, y: u32::from(row) });
        if let Ok(grid) = state.term.grid_ref(point) {
            let mut buf = vec![0u8; 2048];
            if let Ok(n) = grid.hyperlink_uri(&mut buf)
                && n > 0
            {
                buf.truncate(n);
                let url = String::from_utf8_lossy(&buf).into_owned();
                return Some(Link { url, row, start: col, end: col + 1 });
            }
        }
        state.update_frame();
        let r = state.frame.rows.get(usize::from(row))?;
        links::url_at(r, col).map(|(url, start, end)| Link { url, row, start, end })
    }

    /// The scrollback and screen as text, oldest line first, for tests and session restore.
    pub fn contents(&self) -> String {
        let mut formatter_text = String::new();
        let state = self.0.state.borrow();
        if let Ok(all) = state.term.select_all()
            && let Some(sel) = all.as_ref()
        {
            let options = FormatOptions::new()
                .with_emit_format(Format::Plain)
                .with_unwrap(false)
                .with_trim(true)
                .with_selection(sel);
            if let Ok(Some(bytes)) = state.term.format_selection_alloc(None, options) {
                formatter_text = String::from_utf8_lossy(&bytes).into_owned();
            }
        }
        formatter_text
    }
}

/// Unregisters an output waker when dropped.
pub struct WakerGuard {
    session: std::rc::Weak<Inner>,
    id: u64,
}

impl Drop for WakerGuard {
    fn drop(&mut self) {
        if let Some(inner) = self.session.upgrade()
            && let Some(t) = &inner.transport
        {
            t.shared.remove_waker(self.id);
        }
    }
}

impl State {
    fn send(&mut self, bytes: Vec<u8>) {
        match &mut self.sink {
            Sink::Transport(tx) => {
                let _ = tx.send(bytes);
            }
            Sink::Captured(out) => out.extend_from_slice(&bytes),
        }
    }

    /// The viewport cell under a pixel position, clamped into the grid.
    fn point_at(&self, x: f32, y: f32) -> Point {
        let col = (x / self.size.cell_width.max(1) as f32).floor().clamp(0., f32::from(self.size.cols - 1)) as u16;
        let row = (y / self.size.cell_height.max(1) as f32).floor().clamp(0., f32::from(self.size.rows - 1)) as u32;
        Point::Viewport(PointCoordinate { x: col, y: row })
    }

    fn encode_mouse(&mut self, action: mouse::Action, button: Option<mouse::Button>, x: f32, y: f32, mods: Mods) {
        let size = mouse::EncoderSize {
            screen_width: u32::from(self.size.cols) * self.size.cell_width,
            screen_height: u32::from(self.size.rows) * self.size.cell_height,
            cell_width: self.size.cell_width.max(1),
            cell_height: self.size.cell_height.max(1),
            padding_top: 0,
            padding_bottom: 0,
            padding_right: 0,
            padding_left: 0,
        };
        self.mouse_event.set_action(action).set_button(button).set_mods(mods).set_position(mouse::Position { x, y });
        let mut out = Vec::new();
        let ok = self
            .mouse_encoder
            .set_options_from_terminal(&self.term)
            .set_size(size)
            .set_any_button_pressed(self.reported_button.is_some())
            .set_track_last_cell(true)
            .encode_to_vec(&self.mouse_event, &mut out);
        if ok.is_ok() && !out.is_empty() {
            self.send(out);
        }
    }

    fn update_frame(&mut self) {
        let force = std::mem::take(&mut self.force_full);
        let focused = self.focused;
        let Ok(snapshot) = self.render.update(&self.term) else { return };
        let dirty = snapshot.dirty().unwrap_or(Dirty::Full);
        if dirty == Dirty::Clean && !force && !self.frame.rows.is_empty() {
            return;
        }
        let full = force || dirty == Dirty::Full;
        let Ok(colors) = snapshot.colors() else { return };
        let frame = &mut self.frame;
        frame.background = colors.background.into();
        frame.foreground = colors.foreground.into();
        frame.cols = snapshot.cols().unwrap_or(self.size.cols);
        frame.cursor = snapshot.cursor().ok().and_then(|c| {
            let at = c.viewport.filter(|_| c.visible)?;
            let shape = match c.visual_style {
                _ if !focused => CursorShape::BlockHollow,
                CursorVisualStyle::Bar => CursorShape::Bar,
                CursorVisualStyle::Underline => CursorShape::Underline,
                CursorVisualStyle::BlockHollow => CursorShape::BlockHollow,
                _ => CursorShape::Block,
            };
            let col = if at.at_wide_tail { at.x.saturating_sub(1) } else { at.x };
            Some(Cursor {
                col,
                row: at.y,
                shape,
                color: colors.cursor.map(Into::into),
                wide_tail: at.at_wide_tail,
                blinking: c.blinking,
            })
        });
        if let Ok(bar) = self.term.scrollbar() {
            frame.scroll_offset = bar.offset;
            frame.scroll_total = bar.total;
        }

        let mut cached: HashMap<RowId, Row> = HashMap::new();
        if !full {
            for (id, row) in self.row_ids.drain(..).zip(frame.rows.drain(..)) {
                cached.insert(id, row);
            }
        }
        self.row_ids.clear();
        frame.rows.clear();
        let Ok(mut rows) = self.row_iter.update(&snapshot) else { return };
        let palette = &colors.palette;
        while let Some(row) = rows.next() {
            let Ok(id) = row.id() else { continue };
            let row_dirty = row.dirty().unwrap_or(true);
            if !full
                && !row_dirty
                && let Some(kept) = cached.remove(&id)
            {
                frame.rows.push(kept);
                self.row_ids.push(id);
                continue;
            }
            let mut out = Row { text: String::new(), cells: Vec::with_capacity(usize::from(frame.cols)) };
            if let Ok(mut cells) = self.cell_iter.update(row) {
                while let Some(c) = cells.next() {
                    out.cells.push(cell_of(c, &mut out.text, &mut self.grapheme, palette, frame));
                }
            }
            frame.rows.push(out);
            self.row_ids.push(id);
        }
        frame.generation += 1;
        let _ = snapshot.clean();
    }
}

fn cell_of(
    c: &libghostty_vt::render::CellIteration<'_, '_>,
    text: &mut String,
    grapheme: &mut String,
    palette: &[libghostty_vt::style::RgbColor; 256],
    frame: &Frame,
) -> Cell {
    let mut cell = Cell::default();
    let raw = c.raw_cell().ok();
    match raw.and_then(|r| r.wide().ok()) {
        Some(CellWide::Wide) => cell.attrs.insert(Attrs::WIDE),
        Some(CellWide::SpacerTail | CellWide::SpacerHead) => cell.attrs.insert(Attrs::SPACER),
        _ => {}
    }
    if !cell.attrs.contains(Attrs::SPACER) && c.graphemes_len().unwrap_or(0) > 0 && c.graphemes_utf8(grapheme).is_ok() {
        cell.start = text.len() as u32;
        cell.len = grapheme.len() as u16;
        text.push_str(grapheme);
    }
    cell.fg = c.fg_color().ok().flatten().map(Into::into);
    cell.bg = c.bg_color().ok().flatten().map(Into::into);
    if raw.is_some_and(|r| r.has_styling().unwrap_or(false))
        && let Ok(style) = c.style()
    {
        cell.attrs.set(Attrs::BOLD, style.bold);
        cell.attrs.set(Attrs::ITALIC, style.italic);
        cell.attrs.set(Attrs::FAINT, style.faint);
        cell.attrs.set(Attrs::STRIKETHROUGH, style.strikethrough);
        cell.attrs.set(Attrs::OVERLINE, style.overline);
        cell.attrs.set(Attrs::INVISIBLE, style.invisible);
        cell.attrs.set(Attrs::BLINK, style.blink);
        cell.underline = match style.underline {
            libghostty_vt::style::Underline::Single => Underline::Single,
            libghostty_vt::style::Underline::Double => Underline::Double,
            libghostty_vt::style::Underline::Curly => Underline::Curly,
            libghostty_vt::style::Underline::Dotted => Underline::Dotted,
            libghostty_vt::style::Underline::Dashed => Underline::Dashed,
            _ => Underline::None,
        };
        cell.underline_color = match style.underline_color {
            StyleColor::Rgb(c) => Some(c.into()),
            StyleColor::Palette(i) => Some(palette[usize::from(i.0)].into()),
            StyleColor::None => None,
        };
        if style.inverse {
            let fg = cell.fg.unwrap_or(frame.foreground);
            let bg = cell.bg.unwrap_or(frame.background);
            cell.fg = Some(bg);
            cell.bg = Some(fg);
        }
    }
    if raw.is_some_and(|r| r.has_hyperlink().unwrap_or(false)) {
        cell.attrs.insert(Attrs::LINK);
    }
    if c.is_selected().unwrap_or(false) {
        cell.attrs.insert(Attrs::SELECTED);
    }
    cell
}

/// A `file://host/path` URL as its path; anything else as it came.
fn cwd_path(pwd: &str) -> String {
    match pwd.strip_prefix("file://") {
        Some(rest) => match rest.find('/') {
            Some(i) => percent_decode(&rest[i..]),
            None => rest.to_owned(),
        },
        None => pwd.to_owned(),
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(&text[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
