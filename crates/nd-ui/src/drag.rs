//! Drag and drop between elements, in one window or across the app's windows (ARCHITECTURE.md
//! 6.1). A source is a `div` with [`Div::draggable`](crate::Div::draggable), or a widget whose
//! parts drag (a source list's rows) and reports them as [`DragSource`]. A target names the
//! kinds it takes with `accepts_drops`. `nd-ui` owns the session: it finds the window and the
//! innermost accepting target under the pointer from the windows' screen frames and the
//! backend's bounds, and reports [`DragOver`] and [`Dropped`] to the target and [`DragEnded`]
//! to the source. A backend may own the pointer for the length of a drag (AppKit's dragging
//! session draws the image across windows); it then reports where the pointer is per window.

use nd_core::{NodeId, Point, Rect, WindowId};
use nd_gesture::GesturePhase;
use nd_gpui::gpui::SharedString;

use crate::App;

/// What is dragged: an app-defined kind targets filter on, and the dragged thing's id.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DragItem {
    pub kind: SharedString,
    pub id: SharedString,
}

impl DragItem {
    pub fn new(kind: impl Into<SharedString>, id: impl Into<SharedString>) -> Self {
        Self { kind: kind.into(), id: id.into() }
    }
}

/// Where a widget that places drops itself (a source list) would put one: the index among the
/// children of `parent`, or among the roots without one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropSlot {
    pub parent: Option<SharedString>,
    pub index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragOverPhase {
    Entered,
    Moved,
    Exited,
}

/// A drag the target accepts is over it.
#[derive(Clone, Debug, PartialEq)]
pub struct DragOver {
    pub item: DragItem,
    pub phase: DragOverPhase,
    /// In the target window's coordinates.
    pub position: Point,
    /// From the target's top-left.
    pub local: Point,
    pub source_window: WindowId,
    pub slot: Option<DropSlot>,
}

/// The pointer was released over the target.
#[derive(Clone, Debug, PartialEq)]
pub struct Dropped {
    pub item: DragItem,
    pub position: Point,
    pub local: Point,
    pub source_window: WindowId,
    pub slot: Option<DropSlot>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DragOutcome {
    /// A target in `window` took it.
    Dropped { window: WindowId },
    /// Escape, or a release over a window where nothing takes it.
    Cancelled,
    /// Released outside every window of the app, at `screen` (global points, top-left origin).
    Outside { screen: Point },
}

/// Reported to the source once its drag is over.
#[derive(Clone, Debug, PartialEq)]
pub struct DragEnded {
    pub item: DragItem,
    pub outcome: DragOutcome,
}

/// Where a pointer reported by a widget source is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DragPoint {
    /// In the coordinates of the source's window.
    Window(Point),
    /// Global screen points, top-left origin.
    Screen(Point),
}

/// A backend reports a drag from a part of a widget (a source list row) to the widget's node.
/// `native` says the backend owns the pointer meanwhile: moves then arrive through
/// [`App::drag_moved`] from whichever window the pointer is over, and the release through
/// [`App::drag_released`].
#[derive(Clone, Debug, PartialEq)]
pub struct DragSource {
    pub item: DragItem,
    pub phase: GesturePhase,
    pub at: DragPoint,
    pub native: bool,
}

/// What a node does in drags: the item it drags as, and the kinds it takes.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DragSpec {
    pub item: Option<DragItem>,
    pub accepts: Vec<SharedString>,
}

pub(crate) struct Session {
    item: DragItem,
    source_window: WindowId,
    source: NodeId,
    native: bool,
    over: Option<(WindowId, NodeId)>,
    /// Where the target over said the drop would land.
    slot: Option<DropSlot>,
    /// Where the pointer last was, in screen points.
    screen: Option<Point>,
    /// The source is shown as a picture following the pointer outside its window.
    previewing: bool,
}

fn contains(r: &Rect, p: Point) -> bool {
    p.x >= r.origin.x && p.y >= r.origin.y && p.x < r.origin.x + r.size.width && p.y < r.origin.y + r.size.height
}

impl App {
    /// Starts a drag of `item` from `source` in `window`. A drag already running is cancelled.
    pub fn begin_drag(&mut self, window: WindowId, source: NodeId, item: DragItem, native: bool) {
        if self.drag.is_some() {
            self.cancel_drag();
        }
        self.drag = Some(Session {
            item,
            source_window: window,
            source,
            native,
            over: None,
            slot: None,
            screen: None,
            previewing: false,
        });
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Whether the running drag is over a target that takes it.
    pub fn drag_has_target(&self) -> bool {
        self.drag.as_ref().is_some_and(|s| s.over.is_some())
    }

    /// Escape ends the running drag, unless the backend owns it (its session takes the key).
    pub(crate) fn drag_escapes(&self) -> bool {
        self.drag.as_ref().is_some_and(|s| !s.native)
    }

    /// The pointer of the running drag is at `at` in `window`'s coordinates.
    pub fn drag_moved(&mut self, window: WindowId, at: Point) {
        let Some(screen) = self.window_frame(window).map(|f| Point { x: f.origin.x + at.x, y: f.origin.y + at.y })
        else {
            return;
        };
        let Some(session) = self.drag.as_mut() else { return };
        session.screen = Some(screen);
        let target = if session.native { Some((window, at)) } else { self.window_at(screen, window) };
        let hit = target.and_then(|(w, p)| self.target_at(w, p).map(|(n, local)| (w, n, p, local)));
        self.hover(hit);
        self.preview(target.map(|(w, _)| w), screen);
    }

    /// The pointer of a drag the backend owns left every window.
    pub fn drag_exited(&mut self) {
        self.hover(None);
    }

    /// The pointer of the running drag was released at `at` in `window`'s coordinates. Returns
    /// whether a target took it.
    pub fn drag_released(&mut self, window: WindowId, at: Point) -> bool {
        self.drag_moved(window, at);
        let Some(session) = self.drag.take() else { return false };
        self.end_preview(&session);
        let screen = session.screen.unwrap_or(at);
        if let Some((w, node)) = session.over {
            let _ = self.backend.drop_slot(node, None, &session.item);
            let position =
                self.window_frame(w).map_or(at, |f| Point { x: screen.x - f.origin.x, y: screen.y - f.origin.y });
            let local = self.local(node, position);
            let dropped = Dropped {
                item: session.item.clone(),
                position,
                local,
                source_window: session.source_window,
                slot: session.slot.clone(),
            };
            self.events().push(node, Box::new(dropped));
            self.end(session, DragOutcome::Dropped { window: w });
            return true;
        }
        let outcome = if self.window_at(screen, window).is_some() {
            DragOutcome::Cancelled
        } else {
            DragOutcome::Outside { screen }
        };
        self.end(session, outcome);
        false
    }

    /// A drag the backend owns ended at `screen` without a target taking it (a release outside
    /// the app's windows, or Escape).
    pub fn drag_finished(&mut self, screen: Point) {
        let Some(session) = self.drag.take() else { return };
        if let Some((_, node)) = session.over {
            let _ = self.backend.drop_slot(node, None, &session.item);
        }
        self.end_preview(&session);
        let inside = self.window_ids().any(|w| self.window_frame(w).is_some_and(|f| contains(&f, screen)));
        let outcome = if inside { DragOutcome::Cancelled } else { DragOutcome::Outside { screen } };
        self.end(session, outcome);
    }

    /// Escape: the drag ends where it started.
    pub fn cancel_drag(&mut self) {
        self.hover(None);
        let Some(session) = self.drag.take() else { return };
        self.end_preview(&session);
        self.end(session, DragOutcome::Cancelled);
    }

    fn end(&mut self, session: Session, outcome: DragOutcome) {
        self.events().push(session.source, Box::new(DragEnded { item: session.item, outcome }));
    }

    /// Moves the hover to `hit` (window, target, position, local), reporting the exit from the
    /// old target and the entry or move over the new one.
    fn hover(&mut self, hit: Option<(WindowId, NodeId, Point, Point)>) {
        let Some(session) = self.drag.as_mut() else { return };
        let item = session.item.clone();
        let source_window = session.source_window;
        let old = session.over;
        session.over = hit.map(|(w, n, _, _)| (w, n));
        session.slot = None;
        if let Some((_, node)) = old
            && old != hit.map(|(w, n, _, _)| (w, n))
        {
            let _ = self.backend.drop_slot(node, None, &item);
            self.events().push(
                node,
                Box::new(DragOver {
                    item: item.clone(),
                    phase: DragOverPhase::Exited,
                    position: Point::default(),
                    local: Point::default(),
                    source_window,
                    slot: None,
                }),
            );
        }
        if let Some((w, node, position, local)) = hit {
            let phase = if old == Some((w, node)) { DragOverPhase::Moved } else { DragOverPhase::Entered };
            let slot = self.backend.drop_slot(node, Some(position), &item);
            if let Some(session) = self.drag.as_mut() {
                session.slot = slot.clone();
            }
            self.events().push(node, Box::new(DragOver { item, phase, position, local, source_window, slot }));
        }
    }

    /// Shows the source as a picture under the pointer while the pointer is outside its window,
    /// for a drag the backend does not draw itself.
    fn preview(&mut self, over: Option<WindowId>, screen: Point) {
        let Some(session) = self.drag.as_mut() else { return };
        if session.native {
            return;
        }
        let outside = over != Some(session.source_window);
        if outside || session.previewing {
            session.previewing = outside;
            let (w, n) = (session.source_window, session.source);
            self.backend.drag_preview(w, n, outside.then_some(screen));
        }
    }

    fn end_preview(&mut self, session: &Session) {
        if session.previewing {
            self.backend.drag_preview(session.source_window, session.source, None);
        }
    }

    fn window_frame(&self, window: WindowId) -> Option<Rect> {
        self.backend.window_state(window).and_then(|s| s.frame)
    }

    /// The window under `screen` and the point in its coordinates. `prefer` wins where windows
    /// overlap, since nd does not know their stacking order.
    fn window_at(&self, screen: Point, prefer: WindowId) -> Option<(WindowId, Point)> {
        let at = |w: WindowId| {
            let state = self.backend.window_state(w)?;
            let frame = state.frame.filter(|f| state.visible && contains(f, screen))?;
            Some((w, Point { x: screen.x - frame.origin.x, y: screen.y - frame.origin.y }))
        };
        at(prefer).or_else(|| self.window_ids().filter(|w| *w != prefer).find_map(at))
    }

    fn local(&self, node: NodeId, at: Point) -> Point {
        let origin = self.backend.probe(node).bounds.map_or(Point::default(), |b| b.origin);
        Point { x: at.x - origin.x, y: at.y - origin.y }
    }

    /// The innermost node in `window` that takes the running drag's kind and is under `at`,
    /// with `at` from its top-left.
    fn target_at(&self, window: WindowId, at: Point) -> Option<(NodeId, Point)> {
        let kind = &self.drag.as_ref()?.item.kind;
        let tree = self.windows.get(&window)?.tree.as_ref()?;
        tree.drop_targets(kind)
            .into_iter()
            .filter_map(|n| {
                let probe = self.backend.probe(n);
                let b = probe.bounds.filter(|b| probe.visible != Some(false) && contains(b, at))?;
                Some((n, b))
            })
            .min_by(|a, b| {
                let area = |r: &Rect| r.size.width * r.size.height;
                area(&a.1).total_cmp(&area(&b.1))
            })
            .map(|(n, b)| (n, Point { x: at.x - b.origin.x, y: at.y - b.origin.y }))
    }

    /// Runs the session for a drag event on a `draggable` node, or a widget's [`DragSource`].
    /// Returns whether the event belonged to drag and drop.
    pub(crate) fn drag_event(&mut self, node: NodeId, event: &dyn std::any::Any) -> bool {
        let window =
            self.windows.iter().find(|(_, s)| s.tree.as_ref().is_some_and(|t| t.contains(node))).map(|(w, _)| *w);
        let Some(window) = window else { return false };
        if let Some(source) = event.downcast_ref::<DragSource>() {
            let at = |app: &App| match source.at {
                DragPoint::Window(p) => Some(p),
                DragPoint::Screen(s) => {
                    app.window_frame(window).map(|f| Point { x: s.x - f.origin.x, y: s.y - f.origin.y })
                }
            };
            match source.phase {
                GesturePhase::Began => self.begin_drag(window, node, source.item.clone(), source.native),
                GesturePhase::Changed if !source.native => {
                    if let Some(p) = at(self) {
                        self.drag_moved(window, p);
                    }
                }
                GesturePhase::Changed => {}
                GesturePhase::Ended | GesturePhase::Cancelled if source.native => {
                    if self.drag.as_ref().is_some_and(|s| s.source == node) {
                        let screen = match source.at {
                            DragPoint::Screen(s) => s,
                            DragPoint::Window(p) => self
                                .window_frame(window)
                                .map_or(p, |f| Point { x: f.origin.x + p.x, y: f.origin.y + p.y }),
                        };
                        self.drag_finished(screen);
                    }
                }
                GesturePhase::Ended => {
                    if let Some(p) = at(self) {
                        self.drag_released(window, p);
                    }
                }
                GesturePhase::Cancelled => self.cancel_drag(),
            }
            return true;
        }
        let Some(drag) = event.downcast_ref::<nd_gesture::DragEvent>() else { return false };
        let Some(item) = self.windows[&window].tree.as_ref().and_then(|t| t.drag_item(node)) else { return false };
        match drag.phase {
            GesturePhase::Began => {
                self.begin_drag(window, node, item, false);
                self.drag_moved(window, drag.position);
            }
            GesturePhase::Changed => {
                if self.drag.as_ref().is_some_and(|s| s.source == node) {
                    self.drag_moved(window, drag.position);
                }
            }
            GesturePhase::Ended => {
                if self.drag.as_ref().is_some_and(|s| s.source == node) {
                    self.drag_released(window, drag.position);
                }
            }
            GesturePhase::Cancelled => {
                if self.drag.as_ref().is_some_and(|s| s.source == node) {
                    self.cancel_drag();
                }
            }
        }
        // The node's own `.on_drag` still sees the gesture.
        false
    }
}
