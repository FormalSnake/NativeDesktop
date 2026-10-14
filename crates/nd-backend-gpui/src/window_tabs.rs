//! Window tab groups on GPUI (ARCHITECTURE.md 5.5). App windows naming the same `tab_group`
//! share one GPUI window, a group window, which draws the drawn tab bar ([`crate::tab_bar`])
//! and the selected tab's tree. Which tabs a group window holds and which one shows is host
//! presentation state: selecting, reordering and moving a tab to another group window never
//! reach the app, which only sees the moved window resize. Closes and new tabs reach the app on
//! the host's next pump (3.13).
//!
//! The bar hides while a group window holds one tab, as `AdwTabBar`'s autohide does. It sits
//! under the selected tab's title row (its toolbar or titlebar) when it has one, else at the top
//! of the window, where it keeps clear of the window controls and moves the window.
//!
//! Tear-off is drop-to-new-window everywhere: GPUI has no `xdg-toplevel-drag`, so a torn tab
//! is a drawn preview following the pointer until release, where a new group window opens, or
//! the tab joins the bar of another window of its group.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use nd_core::{NodeId, Point, Rect, Size, WindowId};
use nd_gpui::gpui::{
    self, AnyElement, AnyWindowHandle, DispatchPhase, IntoElement, Keystroke, MouseMoveEvent, MouseUpEvent,
    ParentElement, SharedString, Styled, div, px,
};
use nd_ui::TitlebarInsets;
use nd_ui::backend::{NodeKind, WindowCommand};
use nd_ui::semantics::{Paint, PaintKind};

use crate::host::{Inner, NdView};
use crate::nav::{Marks, OnInput};
use crate::tab_bar::{
    Bar, BarInput, BarLayout, BarTab, LayoutSink, OverviewInput, TabLook, contains, drag_preview, overview_view,
    tab_bar_view,
};

/// A GPUI window (or, on a single canvas, an in-canvas window) showing a tab group.
pub(crate) type SurfaceId = u64;

/// What the backend seam shares with the host about tabs.
#[derive(Default)]
pub(crate) struct TabSeam {
    /// Window commands the app sent, for the host's next pump.
    pub commands: Vec<(WindowId, WindowCommand)>,
    /// Tabs a group window holds but does not show.
    pub hidden: HashSet<WindowId>,
    /// Nodes detached from a window, by that window, until they are attached again.
    pub detached: HashMap<NodeId, WindowId>,
    /// Where each drawn tab strip's parts were painted.
    pub strips: HashMap<NodeId, LayoutSink>,
}

pub(crate) struct Surface {
    pub group: SharedString,
    pub tabs: Vec<WindowId>,
    pub selected: WindowId,
    pub overview: bool,
    pub handle: Option<AnyWindowHandle>,
    pub view: Option<gpui::Entity<NdView>>,
    /// When the user last used the window, which a new tab of the group joins.
    touched: u64,
    /// The GPUI window's bounds on screen as of its last draw.
    bounds: Option<Rect>,
    layout: LayoutSink,
    /// The tab order `layout` was painted in.
    painted: Vec<WindowId>,
    /// Tab titles as of the last draw that could read the app.
    titles: HashMap<WindowId, SharedString>,
    /// The title the GPUI window was given.
    shown_title: Option<SharedString>,
}

/// A press on a tab, which may become a reorder or a tear-off.
struct Press {
    surface: SurfaceId,
    tab: WindowId,
    start: Point,
    at: Point,
    /// Where in the tab the pointer took it.
    grab: Point,
    width: f32,
    dragging: bool,
    tearing: bool,
}

pub(crate) enum Request {
    Close(WindowId),
    /// The close button of a group window: every tab, the selected one first.
    CloseAll(SurfaceId),
    NewTab(WindowId),
    Move {
        tab: WindowId,
        to: SurfaceId,
        index: usize,
    },
    /// Opens a group window at `at` (screen coordinates) for the tab.
    TearOff {
        tab: WindowId,
        at: Point,
        size: Size,
    },
}

#[derive(Default)]
pub(crate) struct Groups {
    pub surfaces: BTreeMap<SurfaceId, Surface>,
    next: SurfaceId,
    clock: u64,
    pub requests: Vec<Request>,
    press: Option<Press>,
}

/// Moves the pointer must make before a press on a tab drags it.
const DRAG_START: f32 = 4.;

impl Groups {
    pub fn surface_of(&self, tab: WindowId) -> Option<SurfaceId> {
        self.surfaces.iter().find(|(_, s)| s.tabs.contains(&tab)).map(|(id, _)| *id)
    }

    /// The group window of `group` the user used last.
    pub fn active(&self, group: &SharedString) -> Option<SurfaceId> {
        self.surfaces.iter().filter(|(_, s)| &s.group == group).max_by_key(|(_, s)| s.touched).map(|(id, _)| *id)
    }

    pub fn touch(&mut self, surface: SurfaceId) {
        self.clock += 1;
        if let Some(s) = self.surfaces.get_mut(&surface) {
            s.touched = self.clock;
        }
    }

    pub fn create(&mut self, group: SharedString, tab: WindowId) -> SurfaceId {
        self.next += 1;
        let id = self.next;
        self.surfaces.insert(
            id,
            Surface {
                group,
                tabs: vec![tab],
                selected: tab,
                overview: false,
                handle: None,
                view: None,
                touched: 0,
                bounds: None,
                layout: LayoutSink::default(),
                painted: Vec::new(),
                titles: HashMap::new(),
                shown_title: None,
            },
        );
        self.touch(id);
        id
    }

    /// Adds `tab` to the surface at `index`, selected.
    pub fn attach(&mut self, surface: SurfaceId, tab: WindowId, index: usize) {
        if let Some(s) = self.surfaces.get_mut(&surface) {
            s.tabs.retain(|t| *t != tab);
            s.tabs.insert(index.min(s.tabs.len()), tab);
            s.selected = tab;
        }
        self.touch(surface);
    }

    /// Takes `tab` out of its surface, selecting the tab after it (or before, at the end) when
    /// it was selected. The surface, and whether it is now empty.
    pub fn detach(&mut self, tab: WindowId) -> Option<(SurfaceId, bool)> {
        let id = self.surface_of(tab)?;
        let s = self.surfaces.get_mut(&id)?;
        let at = s.tabs.iter().position(|t| *t == tab)?;
        s.tabs.remove(at);
        if s.selected == tab
            && let Some(next) = s.tabs.get(at).or_else(|| s.tabs.last())
        {
            s.selected = *next;
        }
        Some((id, s.tabs.is_empty()))
    }

    pub fn select(&mut self, surface: SurfaceId, tab: WindowId) {
        if let Some(s) = self.surfaces.get_mut(&surface)
            && s.tabs.contains(&tab)
        {
            s.selected = tab;
        }
        self.touch(surface);
    }

    pub fn hidden(&self) -> HashSet<WindowId> {
        self.surfaces.values().flat_map(|s| s.tabs.iter().copied().filter(move |t| *t != s.selected)).collect()
    }
}

/// A window's tab group as the host shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct TabGroup {
    pub group: SharedString,
    /// In bar order.
    pub tabs: Vec<WindowId>,
    pub selected: WindowId,
    pub overview: bool,
    /// Where the bar and the overview were painted, in window coordinates; empty while the bar
    /// is hidden.
    pub layout: BarLayout,
}

impl Groups {
    pub fn info(&self, tab: WindowId) -> Option<TabGroup> {
        let s = self.surfaces.get(&self.surface_of(tab)?)?;
        Some(TabGroup {
            group: s.group.clone(),
            tabs: s.tabs.clone(),
            selected: s.selected,
            overview: s.overview,
            layout: if s.tabs.len() > 1 { s.layout.borrow().clone() } else { BarLayout::default() },
        })
    }
}

/// The OS close of a GPUI window as nd's drawn window controls press it: through the host,
/// which asks the app (3.13), when one runs.
pub(crate) type CloseFn = dyn Fn(AnyWindowHandle, &mut gpui::App);

pub(crate) struct CloseHook(pub Rc<CloseFn>);

impl gpui::Global for CloseHook {}

pub(crate) fn user_close(window: &mut gpui::Window, cx: &mut gpui::App) {
    match cx.try_global::<CloseHook>().map(|h| h.0.clone()) {
        Some(close) => close(window.window_handle(), cx),
        None => window.remove_window(),
    }
}

pub(crate) fn is_title_row(node: &crate::backend::Node) -> bool {
    matches!(node.kind, NodeKind::Widget(w)
        if w.type_id == std::any::TypeId::of::<nd_components::Toolbar>()
            || w.type_id == std::any::TypeId::of::<nd_components::Titlebar>())
}

fn point_of(p: gpui::Point<gpui::Pixels>) -> Point {
    Point { x: p.x.as_f32(), y: p.y.as_f32() }
}

impl Inner {
    /// Tells the backend which tabs are hidden, after the shown tabs changed.
    pub(crate) fn refresh_hidden(&self) {
        let hidden = self.groups.borrow().hidden();
        self.backend.tab_seam.borrow_mut().hidden = hidden;
    }

    pub(crate) fn notify_surface(&self, surface: SurfaceId, cx: &mut gpui::App) {
        let view = self.groups.borrow().surfaces.get(&surface).and_then(|s| s.view.clone());
        if let Some(view) = view {
            view.update(cx, |_, cx| cx.notify());
        }
    }

    /// Whether the window's tree has a title row for the bar to sit under.
    fn has_title_row(&self, window: WindowId) -> bool {
        let state = self.backend.state.borrow();
        let Some(root) = state.windows.get(&window).and_then(|w| w.root) else { return false };
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let Some(node) = state.nodes.get(&id) else { continue };
            if is_title_row(node) {
                return true;
            }
            stack.extend(node.children.iter().copied());
        }
        false
    }

    /// A group window's GPUI root.
    pub(crate) fn render_group(
        self: &Rc<Self>,
        surface: SurfaceId,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> AnyElement {
        let viewport = window.viewport_size();
        let size = Size { width: viewport.width.as_f32(), height: viewport.height.as_f32() };
        let Some(selected) = self.groups.borrow().surfaces.get(&surface).map(|s| s.selected) else {
            return div().into_any_element();
        };
        let insets =
            self.hooks.titlebar_insets(selected, window).unwrap_or_else(|| crate::host::default_insets(window));
        let content = self.group_content(surface, size, insets, window, cx);
        let title = self.groups.borrow().surfaces.get(&surface).and_then(|s| s.titles.get(&selected).cloned());
        let mut groups = self.groups.borrow_mut();
        if let Some(s) = groups.surfaces.get_mut(&surface)
            && let Some(title) = title
            && s.shown_title.as_ref() != Some(&title)
        {
            window.set_window_title(&title);
            s.shown_title = Some(title);
        }
        content
    }

    /// The group's bar over its selected tab, at `size`, with the overview and a torn tab's
    /// preview above. On a single canvas this is an in-canvas window's content.
    pub(crate) fn group_content(
        self: &Rc<Self>,
        surface: SurfaceId,
        size: Size,
        insets: TitlebarInsets,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> AnyElement {
        let b = window.bounds();
        let (tabs, selected, overview, layout) = {
            let mut groups = self.groups.borrow_mut();
            let Some(s) = groups.surfaces.get_mut(&surface) else { return div().into_any_element() };
            s.bounds = Some(Rect::new(
                b.origin.x.as_f32(),
                b.origin.y.as_f32(),
                b.size.width.as_f32(),
                b.size.height.as_f32(),
            ));
            s.painted = s.tabs.clone();
            (s.tabs.clone(), s.selected, s.overview, s.layout.clone())
        };
        let new_tab = match self.app.try_borrow() {
            Ok(app) => {
                let titles: Vec<(WindowId, SharedString)> =
                    tabs.iter().filter_map(|t| Some((*t, app.window(*t)?.title().clone()))).collect();
                if let Some(s) = self.groups.borrow_mut().surfaces.get_mut(&surface) {
                    s.titles.extend(titles);
                }
                Some(app.window(selected).is_some_and(|w| w.handles_new_tab()))
            }
            Err(_) => None,
        };
        let (titles, new_tab) = {
            let mut groups = self.groups.borrow_mut();
            let Some(s) = groups.surfaces.get_mut(&surface) else { return div().into_any_element() };
            let titles: Vec<SharedString> = tabs.iter().map(|t| s.titles.get(t).cloned().unwrap_or_default()).collect();
            (titles, new_tab.unwrap_or(false))
        };
        let theme = self.backend.state.borrow().theme.window_theme(selected).clone();
        let look = TabLook::of(&theme);
        let shown = tabs.len() > 1;
        let under_title = shown && self.has_title_row(selected);
        let marks = Marks::default();
        let index = tabs.iter().position(|t| *t == selected);
        let bar_tabs: Vec<BarTab> = titles.iter().map(|t| BarTab { title: t.clone(), closable: true }).collect();
        let bar = shown.then(|| {
            let bar = Bar {
                tabs: bar_tabs.clone(),
                selected: index,
                add: new_tab,
                overview: true,
                enabled: true,
                insets: if under_title { (0., 0.) } else { (insets.leading, insets.trailing) },
                drag_region: !under_title,
                label: None,
            };
            tab_bar_view(
                "nd-tabbar",
                &bar,
                &theme,
                Some(self.bar_input(surface, tabs.clone())),
                Some(layout.clone()),
                &marks,
            )
        });
        if !shown {
            *layout.borrow_mut() = BarLayout::default();
        }
        let (content, top) = if under_title {
            (self.render_window_at(selected, size, insets, bar, window, cx), None)
        } else if shown {
            let size = Size { width: size.width, height: (size.height - look.bar_height()).max(1.) };
            (self.render_window_at(selected, size, TitlebarInsets::default(), None, window, cx), bar)
        } else {
            (self.render_window_at(selected, size, insets, None, window, cx), None)
        };
        let overview = (overview && shown).then(|| {
            let weak = Rc::downgrade(self);
            let tabs = tabs.clone();
            let on_input: OnInput<OverviewInput> = Rc::new(move |input, window, _| {
                let Some(inner) = weak.upgrade() else { return };
                let mut groups = inner.groups.borrow_mut();
                if let OverviewInput::Pick(i) = input
                    && let Some(tab) = tabs.get(i)
                {
                    groups.select(surface, *tab);
                }
                if let Some(s) = groups.surfaces.get_mut(&surface) {
                    s.overview = false;
                }
                drop(groups);
                inner.refresh_hidden();
                window.refresh();
            });
            overview_view("nd-tabbar", &bar_tabs, index, &theme, Some(on_input), Some(layout.clone()), &marks)
        });
        if overview.is_none() {
            layout.borrow_mut().cards.clear();
        }
        let preview = {
            let groups = self.groups.borrow();
            groups.press.as_ref().filter(|p| p.surface == surface && p.tearing).map(|p| {
                let title = groups.surfaces[&surface].titles.get(&p.tab).cloned().unwrap_or_default();
                let at = gpui::point(px(p.at.x - p.grab.x), px(p.at.y - p.grab.y));
                drag_preview(&title, &theme, at, p.width)
            })
        };
        self.record_tab_marks(selected, marks, &theme);
        let listener = shown.then(|| self.drag_listener(surface));
        let mut root = div().size_full().relative().flex().flex_col();
        root = match top {
            Some(bar) => root.child(bar).child(div().flex_1().min_h_0().w_full().child(content)),
            None => root.child(content),
        };
        root.children(overview).children(preview).children(listener).into_any_element()
    }

    /// The bar's colours, for the window's appearance audit beside what its tree painted.
    fn record_tab_marks(&self, window: WindowId, marks: Marks, theme: &nd_theme::Theme) {
        let marks = marks.take();
        if marks.is_empty() {
            return;
        }
        let mut state = self.backend.state.borrow_mut();
        let paints = state.paints.entry(window).or_default();
        for mark in marks {
            let background = mark.under.iter().fold(theme.surface.base, |b, f| nd_theme::color::over(*f, b));
            let kind = match mark.text {
                Some((size, weight)) => PaintKind::Text { size, weight },
                None => PaintKind::Fill,
            };
            paints.push(Paint {
                node: None,
                kind,
                color: mark.color,
                background,
                scopes: Vec::new(),
                label: mark.label,
            });
        }
    }

    fn bar_input(self: &Rc<Self>, surface: SurfaceId, tabs: Vec<WindowId>) -> OnInput<BarInput> {
        let weak = Rc::downgrade(self);
        Rc::new(move |input, window, cx| {
            let Some(inner) = weak.upgrade() else { return };
            let Some(tab) = (match input {
                BarInput::Press(i, _) | BarInput::Close(i) => tabs.get(i).copied(),
                BarInput::Add | BarInput::Overview => tabs.first().copied(),
            }) else {
                return;
            };
            let mut groups = inner.groups.borrow_mut();
            match input {
                BarInput::Press(i, at) => {
                    groups.select(surface, tab);
                    let rect = groups.surfaces[&surface].layout.borrow().tabs.get(i).copied().unwrap_or_default();
                    let at = point_of(at);
                    groups.press = Some(Press {
                        surface,
                        tab,
                        start: at,
                        at,
                        grab: Point { x: at.x - rect.origin.x, y: at.y - rect.origin.y },
                        width: rect.size.width,
                        dragging: false,
                        tearing: false,
                    });
                    drop(groups);
                    inner.refresh_hidden();
                    window.refresh();
                    return;
                }
                BarInput::Close(_) => groups.requests.push(Request::Close(tab)),
                BarInput::Add => {
                    let selected = groups.surfaces.get(&surface).map_or(tab, |s| s.selected);
                    groups.touch(surface);
                    groups.requests.push(Request::NewTab(selected));
                }
                BarInput::Overview => {
                    if let Some(s) = groups.surfaces.get_mut(&surface) {
                        s.overview = !s.overview;
                    }
                    drop(groups);
                    window.refresh();
                    return;
                }
            }
            drop(groups);
            (inner.wake())(cx);
        })
    }

    /// Window-wide pointer listeners for a press that turns into a drag: GPUI hands moves and
    /// the release to the window that took the press, wherever the pointer is.
    fn drag_listener(self: &Rc<Self>, surface: SurfaceId) -> AnyElement {
        let (moved, released) = (Rc::downgrade(self), Rc::downgrade(self));
        gpui::canvas(
            |_, _, _| (),
            move |_, (), window, _| {
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _| {
                    if phase == DispatchPhase::Capture
                        && let Some(inner) = moved.upgrade()
                    {
                        inner.drag_move(surface, event, window);
                    }
                });
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture
                        && event.button == gpui::MouseButton::Left
                        && let Some(inner) = released.upgrade()
                    {
                        inner.drag_up(surface, point_of(event.position), window, cx);
                    }
                });
            },
        )
        .absolute()
        .size_0()
        .into_any_element()
    }

    fn drag_move(&self, surface: SurfaceId, event: &MouseMoveEvent, window: &mut gpui::Window) {
        let mut groups = self.groups.borrow_mut();
        let Some(press) = groups.press.as_mut().filter(|p| p.surface == surface) else { return };
        if event.pressed_button != Some(gpui::MouseButton::Left) {
            groups.press = None;
            window.refresh();
            return;
        }
        let at = point_of(event.position);
        press.at = at;
        if !press.dragging && (at.x - press.start.x).hypot(at.y - press.start.y) < DRAG_START {
            return;
        }
        press.dragging = true;
        let tab = press.tab;
        let Some(s) = groups.surfaces.get(&surface) else { return };
        let layout = s.layout.borrow().clone();
        let Some(bar) = layout.bar else { return };
        // Out of the bar by more than a tab's height, the tab tears off.
        let margin = bar.size.height;
        let near = Rect::new(
            bar.origin.x - margin,
            bar.origin.y - margin,
            bar.size.width + 2. * margin,
            bar.size.height + 2. * margin,
        );
        let tearing = !contains(&near, at.x, at.y);
        let index = s
            .painted
            .iter()
            .zip(&layout.tabs)
            .filter(|(t, r)| **t != tab && r.origin.x + r.size.width / 2. < at.x)
            .count();
        if let Some(press) = groups.press.as_mut() {
            press.tearing = tearing;
        }
        if !tearing && let Some(s) = groups.surfaces.get_mut(&surface) {
            s.tabs.retain(|t| *t != tab);
            s.tabs.insert(index.min(s.tabs.len()), tab);
        }
        window.refresh();
    }

    fn drag_up(&self, surface: SurfaceId, at: Point, window: &mut gpui::Window, cx: &mut gpui::App) {
        let mut groups = self.groups.borrow_mut();
        if groups.press.as_ref().is_none_or(|p| p.surface != surface) {
            return;
        }
        let Some(press) = groups.press.take() else { return };
        window.refresh();
        if !press.tearing {
            return;
        }
        let origin = window.bounds().origin;
        let global = Point { x: origin.x.as_f32() + at.x, y: origin.y.as_f32() + at.y };
        let group = groups.surfaces[&surface].group.clone();
        let target = groups.surfaces.iter().filter(|(id, _)| **id != surface).find_map(|(id, s)| {
            let bounds = s.bounds?;
            let bar = s.layout.borrow().bar.filter(|_| s.tabs.len() > 1)?;
            let (x, y) = (global.x - bounds.origin.x, global.y - bounds.origin.y);
            contains(&bar, x, y).then(|| {
                let layout = s.layout.borrow();
                let index =
                    s.painted.iter().zip(&layout.tabs).filter(|(_, r)| r.origin.x + r.size.width / 2. < x).count();
                (*id, s.group == group, index)
            })
        });
        match target {
            Some((to, true, index)) => groups.requests.push(Request::Move { tab: press.tab, to, index }),
            // Tabs never join another group's bar; the tab stays where it was.
            Some((_, false, _)) => return,
            None if groups.surfaces[&surface].tabs.len() > 1 => {
                let viewport = window.viewport_size();
                let size = Size { width: viewport.width.as_f32(), height: viewport.height.as_f32() };
                let at = Point { x: global.x - press.grab.x, y: global.y - press.grab.y };
                groups.requests.push(Request::TearOff { tab: press.tab, at, size });
            }
            None => return,
        }
        drop(groups);
        (self.wake())(cx);
    }

    /// Esc closes an open overview, before nd dispatch sees the key.
    pub(crate) fn tab_key_first(&self, window: WindowId, keystroke: &Keystroke) -> bool {
        if keystroke.key != "escape" {
            return false;
        }
        let mut groups = self.groups.borrow_mut();
        let Some(id) = groups.surface_of(window) else { return false };
        groups.surfaces.get_mut(&id).is_some_and(|s| std::mem::replace(&mut s.overview, false))
    }

    /// Ctrl-Tab, Ctrl-Shift-Tab and Ctrl-Page Up/Down cycle a group window's tabs, when nd's
    /// bindings left the key alone.
    pub(crate) fn tab_key_last(&self, window: WindowId, keystroke: &Keystroke) -> bool {
        let m = &keystroke.modifiers;
        if !m.control || m.alt || m.platform {
            return false;
        }
        let step: isize = match keystroke.key.as_str() {
            "tab" if m.shift => -1,
            "tab" => 1,
            "pageup" => -1,
            "pagedown" => 1,
            _ => return false,
        };
        let mut groups = self.groups.borrow_mut();
        let Some(id) = groups.surface_of(window) else { return false };
        let s = &groups.surfaces[&id];
        if s.tabs.len() < 2 {
            return false;
        }
        let at = s.tabs.iter().position(|t| *t == s.selected).unwrap_or(0) as isize;
        let next = s.tabs[(at + step).rem_euclid(s.tabs.len() as isize) as usize];
        groups.select(id, next);
        drop(groups);
        self.refresh_hidden();
        true
    }

    /// Window commands and tab requests, on the host's pump with the app free.
    pub(crate) fn tab_requests(self: &Rc<Self>, cx: &mut gpui::App) {
        let commands = std::mem::take(&mut self.backend.tab_seam.borrow_mut().commands);
        for (id, command) in commands {
            let surface = self.groups.borrow().surface_of(id);
            match command {
                WindowCommand::Activate => {
                    if let Some(s) = surface {
                        self.groups.borrow_mut().select(s, id);
                        self.refresh_hidden();
                        self.notify_surface(s, cx);
                    }
                    if let Some(handle) = self.windows.borrow().get(&id).and_then(|w| w.handle) {
                        let _ = handle.update(cx, |_, window, _| window.activate_window());
                    }
                }
                WindowCommand::ToggleTabOverview => {
                    let Some(s) = surface else { continue };
                    if let Some(s) = self.groups.borrow_mut().surfaces.get_mut(&s) {
                        s.overview = !s.overview;
                    }
                    self.notify_surface(s, cx);
                }
                _ => {}
            }
        }
        let requests = std::mem::take(&mut self.groups.borrow_mut().requests);
        for request in requests {
            match request {
                Request::Close(tab) => self.with_app(|app| {
                    app.request_close(tab);
                }),
                Request::CloseAll(surface) => {
                    let tabs = self.groups.borrow().surfaces.get(&surface).map(|s| {
                        let mut tabs = vec![s.selected];
                        tabs.extend(s.tabs.iter().copied().filter(|t| *t != s.selected));
                        tabs
                    });
                    self.with_app(|app| {
                        for tab in tabs.into_iter().flatten() {
                            app.request_close(tab);
                        }
                    });
                }
                Request::NewTab(tab) => self.with_app(|app| {
                    app.request_new_tab(tab);
                }),
                Request::Move { tab, to, index } => self.move_tab(tab, to, index, cx),
                Request::TearOff { tab, at, size } => self.tear_off(tab, at, size, cx),
            }
        }
    }

    fn with_app(&self, f: impl FnOnce(&mut nd_ui::App)) {
        if let Ok(mut app) = self.app.try_borrow_mut() {
            f(&mut app);
            app.flush_effects();
        }
    }

    /// Points the app window at the GPUI window showing `surface`.
    fn rehost(&self, tab: WindowId, surface: SurfaceId) {
        let (handle, view) = {
            let groups = self.groups.borrow();
            let Some(s) = groups.surfaces.get(&surface) else { return };
            (s.handle, s.view.clone())
        };
        if let Some(w) = self.windows.borrow_mut().get_mut(&tab) {
            w.handle = handle;
            w.view = view;
        }
        if let Some(handle) = handle {
            self.backend.set_window_handle(tab, handle);
        }
    }

    /// Drops a surface the last tab left, closing its GPUI window unless others still show
    /// there (a single canvas).
    fn drop_surface(&self, surface: SurfaceId, cx: &mut gpui::App) {
        let Some(s) = self.groups.borrow_mut().surfaces.remove(&surface) else { return };
        if self.single_canvas {
            return;
        }
        if let Some(handle) = s.handle {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }

    fn move_tab(&self, tab: WindowId, to: SurfaceId, index: usize, cx: &mut gpui::App) {
        let from = {
            let mut groups = self.groups.borrow_mut();
            let Some(from) = groups.surface_of(tab) else { return };
            let same_group = groups.surfaces.get(&to).is_some_and(|t| t.group == groups.surfaces[&from].group);
            if !same_group || from == to {
                return;
            }
            let Some((from, empty)) = groups.detach(tab) else { return };
            groups.attach(to, tab, index);
            (from, empty)
        };
        self.rehost(tab, to);
        self.refresh_hidden();
        match from {
            (from, true) => self.drop_surface(from, cx),
            (from, false) => self.notify_surface(from, cx),
        }
        self.notify_surface(to, cx);
    }

    fn tear_off(self: &Rc<Self>, tab: WindowId, at: Point, size: Size, cx: &mut gpui::App) {
        let (from, surface) = {
            let mut groups = self.groups.borrow_mut();
            let Some(from) = groups.surface_of(tab) else { return };
            if groups.surfaces[&from].tabs.len() < 2 {
                return;
            }
            let group = groups.surfaces[&from].group.clone();
            groups.detach(tab);
            (from, groups.create(group, tab))
        };
        if self.single_canvas {
            let canvas = {
                let groups = self.groups.borrow();
                groups.surfaces.get(&from).map(|s| (s.handle, s.view.clone()))
            };
            if let Some((handle, view)) = canvas
                && let Some(s) = self.groups.borrow_mut().surfaces.get_mut(&surface)
            {
                s.handle = handle;
                s.view = view;
            }
        } else {
            let title = self.app.try_borrow().ok().and_then(|app| app.window(tab).map(|w| w.title().clone()));
            let bounds = gpui::Bounds {
                origin: gpui::point(px(at.x), px(at.y)),
                size: gpui::size(px(size.width), px(size.height)),
            };
            self.open_gpui_window(cx, tab, title.unwrap_or_default(), bounds, Some(surface));
        }
        self.rehost(tab, surface);
        self.refresh_hidden();
        self.notify_surface(from, cx);
        self.notify_surface(surface, cx);
    }
}
