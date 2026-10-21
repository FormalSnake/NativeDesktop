//! Entities, contexts and the frame loop (ARCHITECTURE.md 3.1, 3.3, 3.7).
//!
//! The method names and semantics follow GPUI's application core: an entity is leased for the
//! length of an `update`, `cx.notify()` marks a view dirty, observers run when effects flush.

use std::any::{Any, TypeId, type_name};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::rc::{Rc, Weak};

use nd_core::{FrameTime, NodeId, Size, WindowId};
use nd_gpui::gpui::{Action, KeyBinding, Keystroke, SharedString};

use crate::backend::Backend;
use crate::dispatch::{DispatchTree, KeyDispatch};
use crate::element::{AnyElement, IntoElement};
use crate::events::EventQueue;
use crate::overlay::{DismissCause, Dismissed, Modality};
use crate::transaction::MeasureCache;
use crate::tree::Tree;
use crate::window::{Window, WindowHandle, WindowOptions};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId(u64);

impl fmt::Debug for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "e{}", self.0)
    }
}

/// A strong handle. The entity lives while any handle (or a mounted view of it) does.
pub struct Entity<T> {
    id: EntityId,
    alive: Rc<()>,
    _type: PhantomData<fn() -> T>,
}

pub struct WeakEntity<T> {
    id: EntityId,
    alive: Weak<()>,
    _type: PhantomData<fn() -> T>,
}

impl<T> Clone for Entity<T> {
    fn clone(&self) -> Self {
        Self { id: self.id, alive: self.alive.clone(), _type: PhantomData }
    }
}

impl<T> Clone for WeakEntity<T> {
    fn clone(&self) -> Self {
        Self { id: self.id, alive: self.alive.clone(), _type: PhantomData }
    }
}

impl<T> PartialEq for Entity<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T> fmt::Debug for Entity<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Entity<{}>({:?})", type_name::<T>(), self.id)
    }
}

impl<T: 'static> Entity<T> {
    pub fn entity_id(&self) -> EntityId {
        self.id
    }

    pub fn downgrade(&self) -> WeakEntity<T> {
        WeakEntity { id: self.id, alive: Rc::downgrade(&self.alive), _type: PhantomData }
    }

    /// Panics while the entity is leased by an `update`.
    pub fn read<'a>(&self, cx: &'a App) -> &'a T {
        cx.read_entity(self.id)
    }

    pub fn update<R>(&self, cx: &mut App, f: impl FnOnce(&mut T, &mut Context<T>) -> R) -> R {
        cx.lease(self.downgrade(), f)
    }
}

impl<T: 'static> WeakEntity<T> {
    pub fn entity_id(&self) -> EntityId {
        self.id
    }

    pub fn upgrade(&self) -> Option<Entity<T>> {
        Some(Entity { id: self.id, alive: self.alive.upgrade()?, _type: PhantomData })
    }

    /// `None` when the entity is gone.
    pub fn update<R>(&self, cx: &mut App, f: impl FnOnce(&mut T, &mut Context<T>) -> R) -> Option<R> {
        let entity = self.upgrade()?;
        Some(entity.update(cx, f))
    }
}

/// A view: an entity that renders. Rendering one re-runs only its own `render`; child views
/// inside its output are boundaries and render when they are notified themselves.
pub trait Render: 'static + Sized {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement;
}

/// A type-erased view, as it sits in the retained tree.
#[derive(Clone)]
pub struct AnyView {
    id: EntityId,
    _alive: Rc<()>,
    type_id: TypeId,
    render: fn(WeakEntityAny, &mut Window, &mut App) -> AnyElement,
}

#[derive(Clone)]
pub(crate) struct WeakEntityAny {
    id: EntityId,
    alive: Weak<()>,
}

impl AnyView {
    pub fn entity_id(&self) -> EntityId {
        self.id
    }

    pub(crate) fn view_type(&self) -> TypeId {
        self.type_id
    }

    pub(crate) fn render(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        (self.render)(WeakEntityAny { id: self.id, alive: Rc::downgrade(&self._alive) }, window, cx)
    }
}

impl<V: Render> From<Entity<V>> for AnyView {
    fn from(entity: Entity<V>) -> Self {
        fn render<V: Render>(weak: WeakEntityAny, window: &mut Window, cx: &mut App) -> AnyElement {
            let weak = WeakEntity::<V> { id: weak.id, alive: weak.alive, _type: PhantomData };
            cx.lease(weak, |view, cx| view.render(window, cx).into_any_element())
        }
        AnyView { id: entity.id, _alive: entity.alive, type_id: TypeId::of::<V>(), render: render::<V> }
    }
}

impl<V: Render> IntoElement for Entity<V> {
    fn into_any_element(self) -> AnyElement {
        AnyElement::view(self.into())
    }
}

/// Keeps an observation alive; dropping it ends the observation unless detached.
#[must_use = "dropping a Subscription ends it; call detach() to keep it for the entity's life"]
pub struct Subscription {
    active: Rc<Cell<bool>>,
    detached: bool,
}

impl Subscription {
    pub fn detach(mut self) {
        self.detached = true;
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if !self.detached {
            self.active.set(false);
        }
    }
}

type ObserverFn = Rc<RefCell<dyn FnMut(&mut App) -> bool>>;

struct Observer {
    active: Rc<Cell<bool>>,
    callback: ObserverFn,
}

struct Slot {
    value: Option<Box<dyn Any>>,
    alive: Weak<()>,
    type_name: &'static str,
}

pub(crate) struct WindowSlot {
    pub window: Option<Window>,
    pub tree: Option<Tree>,
}

pub struct App {
    entities: HashMap<EntityId, Slot>,
    next_entity: u64,
    next_node: u64,
    next_window: u64,
    notified: VecDeque<EntityId>,
    observers: HashMap<EntityId, Vec<Observer>>,
    globals: HashMap<TypeId, Box<dyn Any>>,
    flushing: bool,
    pub(crate) dirty: HashSet<EntityId>,
    pub(crate) windows: BTreeMap<WindowId, WindowSlot>,
    pub(crate) view_windows: HashMap<EntityId, WindowId>,
    pub(crate) render_counts: HashMap<EntityId, u64>,
    pub(crate) backend: Box<dyn Backend>,
    pub(crate) measure_cache: MeasureCache,
    events: EventQueue,
    bindings: Vec<KeyBinding>,
    /// Set by [`App::propagate`] inside an `on_action` listener.
    pub(crate) propagate_action: bool,
    /// Actions dispatched while their window was lent to a native event handler (a button's
    /// click dispatching a menu command): listeners need that window, so they run once the
    /// handler hands it back.
    deferred_actions: Vec<(WindowId, Box<dyn Action>)>,
    /// Focus requests by element id made while the window's dispatch tree was leased to an action
    /// listener: they land once the tree is back.
    deferred_focus: Vec<(WindowId, String)>,
    /// The views rendering now, innermost last.
    pub(crate) rendering: Vec<EntityId>,
    /// The window whose views are rendering or whose handlers are running.
    pub(crate) current_window: Option<WindowId>,
    /// Views that moved to another window in this frame; a second move is a render bug.
    moved: HashSet<EntityId>,
    /// Frames run so far.
    pub(crate) frames: u64,
    /// Titles set while a handler or a frame held the window, applied once it is back.
    retitled: HashMap<WindowId, SharedString>,
    /// The user asked for less motion: element animations jump instead of moving.
    pub(crate) reduce_motion: bool,
    /// The drag and drop running now.
    pub(crate) drag: Option<crate::drag::Session>,
}

impl App {
    pub fn with_backend(mut backend: Box<dyn Backend>) -> Self {
        let events = EventQueue::default();
        backend.set_event_queue(events.clone());
        Self {
            entities: HashMap::new(),
            next_entity: 1,
            next_node: 1,
            next_window: 1,
            notified: VecDeque::new(),
            observers: HashMap::new(),
            globals: HashMap::new(),
            flushing: false,
            dirty: HashSet::new(),
            windows: BTreeMap::new(),
            view_windows: HashMap::new(),
            render_counts: HashMap::new(),
            backend,
            measure_cache: MeasureCache::default(),
            events,
            bindings: Vec::new(),
            propagate_action: false,
            deferred_actions: Vec::new(),
            deferred_focus: Vec::new(),
            rendering: Vec::new(),
            current_window: None,
            moved: HashSet::new(),
            frames: 0,
            retitled: HashMap::new(),
            reduce_motion: false,
            drag: None,
        }
    }

    /// Follows the user's reduce motion setting: animated offsets, scales and sizes jump to
    /// their targets, and only opacity still fades. Hosts set it from the OS.
    pub fn set_reduce_motion(&mut self, reduce: bool) {
        self.reduce_motion = reduce;
    }

    pub fn reduce_motion(&self) -> bool {
        self.reduce_motion
    }

    /// A backend laid canvas `node` out at `width` x `height`: it paints again for that size.
    fn repaint_canvas(&mut self, node: NodeId, width: f32, height: f32) {
        let Some(id) =
            self.windows.iter().find(|(_, w)| w.tree.as_ref().is_some_and(|t| t.contains(node))).map(|(id, _)| *id)
        else {
            return;
        };
        let Some(mut tree) = self.windows.get_mut(&id).and_then(|w| w.tree.take()) else { return };
        tree.repaint_canvas(node, width, height, self);
        if let Some(slot) = self.windows.get_mut(&id) {
            slot.tree = Some(tree);
        }
    }

    /// Drops every measured size and lays the windows out again in the next frame: what a
    /// backend measures changed without its props (a theme's font).
    pub fn remeasure(&mut self) {
        self.measure_cache = MeasureCache::default();
        for tree in self.windows.values_mut().filter_map(|w| w.tree.as_mut()) {
            tree.remeasure();
        }
    }

    /// GPUI's `cx.new`: builds the entity with a context for itself.
    #[allow(clippy::new_ret_no_self, clippy::wrong_self_convention)]
    pub fn new<T: 'static>(&mut self, build: impl FnOnce(&mut Context<T>) -> T) -> Entity<T> {
        let id = EntityId(self.next_entity);
        self.next_entity += 1;
        let alive = Rc::new(());
        self.entities.insert(id, Slot { value: None, alive: Rc::downgrade(&alive), type_name: type_name::<T>() });
        let entity = Entity { id, alive, _type: PhantomData };
        let mut cx = Context { app: self, entity: entity.downgrade() };
        let value = build(&mut cx);
        self.entities.get_mut(&id).expect("entity slot").value = Some(Box::new(value));
        entity
    }

    fn read_entity<T: 'static>(&self, id: EntityId) -> &T {
        let slot = self.entities.get(&id).unwrap_or_else(|| panic!("{} {id:?} was released", type_name::<T>()));
        let Some(value) = slot.value.as_ref() else {
            panic!("{} {id:?} is being updated and cannot be read", slot.type_name);
        };
        value.downcast_ref().expect("entity type")
    }

    fn lease<T: 'static, R>(&mut self, entity: WeakEntity<T>, f: impl FnOnce(&mut T, &mut Context<T>) -> R) -> R {
        let id = entity.id;
        let slot = self.entities.get_mut(&id).unwrap_or_else(|| panic!("{} {id:?} was released", type_name::<T>()));
        let Some(value) = slot.value.take() else {
            panic!("{} {id:?} is already being updated", slot.type_name);
        };
        let mut value = value.downcast::<T>().expect("entity type");
        let result = f(&mut value, &mut Context { app: self, entity });
        self.entities.get_mut(&id).expect("entity slot").value = Some(value);
        result
    }

    /// Marks the entity changed: its observers run when effects flush, and if it is a mounted
    /// view it re-renders in the next frame.
    /// The view whose render (or a deferred element it returned) is running, if any. A model a
    /// view reads through a deferred element records it, to notify that view when it changes.
    pub fn rendering_view(&self) -> Option<EntityId> {
        self.rendering.last().copied()
    }

    pub fn notify(&mut self, id: EntityId) {
        self.dirty.insert(id);
        self.notified.push_back(id);
    }

    pub fn set_global<G: 'static>(&mut self, global: G) {
        self.globals.insert(TypeId::of::<G>(), Box::new(global));
    }

    pub fn try_global<G: 'static>(&self) -> Option<&G> {
        self.globals.get(&TypeId::of::<G>()).and_then(|g| g.downcast_ref())
    }

    pub fn global<G: 'static>(&self) -> &G {
        self.try_global().unwrap_or_else(|| panic!("no global {}", type_name::<G>()))
    }

    /// The global of type `G`, mutably. Panics when none is set, like [`App::global`].
    pub fn global_mut<G: 'static>(&mut self) -> &mut G {
        self.try_global_mut().unwrap_or_else(|| panic!("no global {}", type_name::<G>()))
    }

    pub fn try_global_mut<G: 'static>(&mut self) -> Option<&mut G> {
        self.globals.get_mut(&TypeId::of::<G>()).and_then(|g| g.downcast_mut())
    }

    /// Runs `f` with the global of type `G` and the app, GPUI's `update_global`. Panics when
    /// none is set. The global is out of the app while `f` runs, so `f` cannot read it again.
    pub fn update_global<G: 'static, R>(&mut self, f: impl FnOnce(&mut G, &mut App) -> R) -> R {
        let mut global =
            self.globals.remove(&TypeId::of::<G>()).unwrap_or_else(|| panic!("no global {}", type_name::<G>()));
        let result = f(global.downcast_mut().expect("global type"), self);
        self.globals.insert(TypeId::of::<G>(), global);
        result
    }

    pub fn has_global<G: 'static>(&self) -> bool {
        self.globals.contains_key(&TypeId::of::<G>())
    }

    /// Runs observers of notified entities and drops entities no handle refers to.
    pub fn flush_effects(&mut self) {
        if self.flushing {
            return;
        }
        self.flushing = true;
        self.retitled.retain(|id, title| match self.windows.get_mut(id) {
            Some(slot) => match slot.window.as_mut() {
                Some(window) => {
                    window.set_title(title.clone());
                    false
                }
                None => true,
            },
            None => false,
        });
        loop {
            while let Some(id) = self.notified.pop_front() {
                self.run_observers(id);
            }
            if !self.release_dropped() && self.notified.is_empty() {
                break;
            }
        }
        self.flushing = false;
    }

    fn run_observers(&mut self, id: EntityId) {
        let Some(list) = self.observers.get_mut(&id) else { return };
        list.retain(|o| o.active.get());
        let callbacks: Vec<(Rc<Cell<bool>>, ObserverFn)> =
            list.iter().map(|o| (o.active.clone(), o.callback.clone())).collect();
        for (active, callback) in callbacks {
            if active.get() && !(callback.borrow_mut())(self) {
                active.set(false);
            }
        }
    }

    fn release_dropped(&mut self) -> bool {
        let dead: Vec<EntityId> = self
            .entities
            .iter()
            .filter(|(_, slot)| slot.value.is_some() && slot.alive.strong_count() == 0)
            .map(|(id, _)| *id)
            .collect();
        for id in &dead {
            // Dropping the value can drop the last handle to another entity; the loop in
            // `flush_effects` comes back for those.
            let slot = self.entities.remove(id);
            self.observers.remove(id);
            self.dirty.remove(id);
            self.render_counts.remove(id);
            drop(slot);
        }
        !dead.is_empty()
    }

    pub(crate) fn alloc_node(&mut self) -> NodeId {
        let id = NodeId(self.next_node);
        self.next_node += 1;
        id
    }

    pub fn open_window<V: Render>(
        &mut self,
        options: WindowOptions,
        build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> WindowHandle<V> {
        let id = WindowId(self.next_window);
        self.next_window += 1;
        let mut window = Window::new(id, &options);
        let root = self.alloc_node();
        let entity = build(&mut window, self);
        let tree = Tree::new(id, root, window.viewport_size(), entity.clone().into(), self.bindings.clone());
        self.dirty.insert(entity.entity_id());
        self.windows.insert(id, WindowSlot { window: Some(window), tree: Some(tree) });
        WindowHandle::new(id, entity)
    }

    /// Adds key bindings for every window, open or opened later.
    pub fn bind_keys(&mut self, bindings: impl IntoIterator<Item = KeyBinding>) {
        let bindings: Vec<KeyBinding> = bindings.into_iter().collect();
        for slot in self.windows.values_mut() {
            if let Some(tree) = slot.tree.as_mut() {
                tree.dispatch.keymap_mut().add_bindings(bindings.iter().cloned());
            }
        }
        self.bindings.extend(bindings);
    }

    /// Every key binding added so far, in order. Menus read their key equivalents here, so a
    /// menu and its shortcut never disagree.
    pub fn key_bindings(&self) -> &[KeyBinding] {
        &self.bindings
    }

    /// The window being rendered, or whose event or action handler is running; `None`
    /// outside any window. Per-window globals (the theme) resolve against it.
    pub fn current_window(&self) -> Option<WindowId> {
        self.current_window
    }

    /// Re-renders every mounted view in the next frame, as after a theme change.
    pub fn refresh_windows(&mut self) {
        let views: Vec<EntityId> = self.view_windows.keys().copied().collect();
        self.dirty.extend(views);
    }

    /// Plays a trackpad haptic, timed to land with the next drawn frame (`Alignment` when a value
    /// snaps, `LevelChange` when a threshold is crossed). A no-op off macOS.
    pub fn haptic(&mut self, haptic: nd_gesture::Haptic) {
        self.backend.haptic(haptic);
    }

    /// Inside an `on_action` listener: lets the action continue towards the root.
    pub fn propagate(&mut self) {
        self.propagate_action = true;
    }

    /// Matches `keystroke` in the window's focused context and dispatches the bound action to
    /// the elements' `on_action` listeners. Platforms call it before their own text handling.
    pub fn dispatch_keystroke(&mut self, window: WindowId, keystroke: &Keystroke) -> KeyDispatch {
        if keystroke.key == "escape" && self.drag_escapes() {
            self.cancel_drag();
            return KeyDispatch::Handled;
        }
        self.with_dispatch(window, KeyDispatch::Unhandled, |tree, app| tree.dispatch_key(keystroke, app))
    }

    /// The app lost activation: every light-dismiss overlay in every window is asked to close.
    pub fn app_deactivated(&mut self) {
        let nodes: Vec<NodeId> = self
            .windows
            .values()
            .filter_map(|w| w.tree.as_ref())
            .flat_map(|t| t.open_overlays().filter(|(_, m)| *m == Modality::LightDismiss).map(|(n, _)| n))
            .collect();
        for node in nodes {
            self.events.push(node, Box::new(Dismissed { reason: DismissCause::Deactivated }));
        }
        self.drain_events();
    }

    /// Open overlays in the window, oldest first, with their modality.
    pub fn open_overlays(&self, window: WindowId) -> Vec<(NodeId, Modality)> {
        self.windows.get(&window).and_then(|w| w.tree.as_ref()).map_or_else(Vec::new, |t| t.open_overlays().collect())
    }

    /// Bubbles `action` from the window's focused node to its root. Menus and toolbar items
    /// dispatch through here. Returns whether a listener handled it.
    pub fn dispatch_action(&mut self, window: WindowId, action: &dyn Action) -> bool {
        if self.windows.get(&window).is_some_and(|slot| slot.window.is_none()) {
            self.deferred_actions.push((window, action.boxed_clone()));
            return true;
        }
        self.with_dispatch(window, false, |tree, app| tree.dispatch_action(action, app))
    }

    /// Runs `f` on the window's dispatch tree with the tree leased out of its slot, so listeners
    /// get the whole app; effects flush afterwards.
    fn with_dispatch<R>(
        &mut self,
        window: WindowId,
        absent: R,
        f: impl FnOnce(&mut DispatchTree<App>, &mut App) -> R,
    ) -> R {
        let Some(mut tree) = self.windows.get_mut(&window).and_then(|w| w.tree.take()) else { return absent };
        let outer = self.current_window.replace(window);
        let ring_before = tree.dispatch.focus_visible();
        let result = f(&mut tree.dispatch, self);
        self.current_window = outer;
        tree.push_focus_change();
        if tree.dispatch.focus_visible() != ring_before {
            // Drawn focus rings read `focus_visible` when they render.
            self.dirty.insert(tree.root_view_id());
        }
        if let Some(button) = tree.dispatch.take_activation() {
            self.events.emitter(button).emit(crate::ClickEvent);
        }
        if let Some(scope) = tree.dispatch.take_dismissal() {
            self.events.emitter(scope).emit(crate::dispatch::DismissRequested);
        }
        if tree.dispatch.take_sidebar_toggle() {
            self.window_command(window, crate::backend::WindowCommand::ToggleSidebar);
        }
        match self.windows.get_mut(&window) {
            Some(slot) => slot.tree = Some(tree),
            // A listener closed the window.
            None => tree.close(self),
        }
        self.flush_effects();
        result
    }

    /// Moves the window's focus to `node`. False when the node is not focusable. The backend
    /// follows in the next frame.
    pub fn focus(&mut self, window: WindowId, node: NodeId) -> bool {
        self.with_dispatch(window, false, |tree, _| tree.focus(node))
    }

    /// Moves the window's focus to the element given `.id(id)`, or to its first focusable
    /// descendant when it is not focusable itself. False when none is mounted or nothing there
    /// takes focus. Inside an action listener the tree is out, so the request waits for it.
    /// The node keyed `id` (`.id(..)`, or a widget's id) in `window` and its laid-out frame,
    /// for anchoring a menu or popover to it from a handler.
    pub fn node_with_id(&self, window: WindowId, id: &str) -> Option<(NodeId, Option<nd_core::Rect>)> {
        self.windows.get(&window)?.tree.as_ref()?.keyed(id)
    }

    pub fn focus_id(&mut self, window: WindowId, id: &str) -> bool {
        if self.windows.get(&window).is_some_and(|w| w.tree.is_none()) {
            self.deferred_focus.push((window, id.to_owned()));
            return true;
        }
        let node = self.windows.get(&window).and_then(|w| w.tree.as_ref()).and_then(|t| t.focus_target(id));
        node.is_some_and(|node| self.focus(window, node))
    }

    pub fn blur(&mut self, window: WindowId) {
        self.with_dispatch(window, (), |tree, _| tree.blur());
    }

    /// A press reached the window: focus rings hide until the keyboard moves focus again.
    /// Backends call it on every primary pointer-down.
    pub fn pointer_down(&mut self, window: WindowId) {
        self.with_dispatch(window, (), |tree, _| tree.set_modality(crate::dispatch::InputModality::Pointer));
    }

    /// Whether the window's focused node shows its focus ring: the keyboard moved last.
    pub fn focus_visible(&self, window: WindowId) -> bool {
        self.windows.get(&window).and_then(|w| w.tree.as_ref()).is_some_and(|t| t.dispatch.focus_visible())
    }

    /// The window's focused node.
    pub fn focused(&self, window: WindowId) -> Option<NodeId> {
        self.windows.get(&window)?.tree.as_ref()?.dispatch.focused()
    }

    /// The window's Tab stops in traversal order.
    pub fn focus_order(&self, window: WindowId) -> Vec<NodeId> {
        self.windows.get(&window).and_then(|w| w.tree.as_ref()).map_or_else(Vec::new, |t| t.dispatch.focus_order())
    }

    /// Records a focus change the platform made itself (a click into a native view). Sends
    /// nothing back to the backend. False when `node` is not focusable in the window.
    pub fn platform_focused(&mut self, window: WindowId, node: Option<NodeId>) -> bool {
        self.with_dispatch(window, false, |tree, _| tree.platform_focused(node))
    }

    /// The user asked to close the window (ARCHITECTURE.md 5.5). Runs its
    /// [`Window::on_should_close`] handler and closes the window unless the handler holds it.
    /// Hosts call it from a later run loop turn, never inside the OS callback. Returns whether
    /// the window closed.
    pub fn request_close(&mut self, id: WindowId) -> bool {
        let handler = self.windows.get(&id).and_then(|w| w.window.as_ref()).and_then(|w| w.should_close.clone());
        let close = match handler {
            Some(handler) => self.with_window(id, true, |window, cx| handler(window, cx)),
            None => true,
        };
        if close && self.windows.contains_key(&id) {
            self.close_window(id);
        }
        self.flush_effects();
        close
    }

    /// The user asked for a new tab next to `id` (the tab bar's `+`). Runs the window's
    /// [`Window::on_new_tab`] handler; false when it has none.
    pub fn request_new_tab(&mut self, id: WindowId) -> bool {
        let handler = self.windows.get(&id).and_then(|w| w.window.as_ref()).and_then(|w| w.new_tab.clone());
        let Some(handler) = handler else { return false };
        self.with_window(id, (), |window, cx| handler(window, cx));
        self.flush_effects();
        true
    }

    /// Asks the platform to do `command` to the window: bring it forward (selecting its tab in a
    /// tab group), or toggle its tab group's overview.
    pub fn window_command(&mut self, id: WindowId, command: crate::backend::WindowCommand) {
        if self.windows.contains_key(&id) {
            self.backend.window_command(id, command);
        }
    }

    /// Sets the window's title (a native tab's title on macOS).
    pub fn set_window_title(&mut self, id: WindowId, title: impl Into<SharedString>) {
        let title = title.into();
        let Some(slot) = self.windows.get_mut(&id) else { return };
        match slot.window.as_mut() {
            Some(window) => window.set_title(title.clone()),
            None => {
                self.retitled.insert(id, title.clone());
            }
        }
        self.backend.set_window_title(id, &title);
    }

    /// Whether the app quits once its last window closes (the default). An app with a tray
    /// item turns it off to keep running behind the item with no window open.
    pub fn set_quit_when_windows_close(&mut self, quit: bool) {
        self.backend.set_quit_when_windows_close(quit);
    }

    /// Ends the app as the OS's Quit does: the way out of an app that outlives its windows.
    pub fn quit(&mut self) {
        self.backend.quit();
    }

    /// Brings the window forward and makes it key; in a tab group its tab is selected.
    pub fn activate_window(&mut self, id: WindowId) {
        self.window_command(id, crate::backend::WindowCommand::Activate);
    }

    /// Runs `f` with the window leased out of its slot, as an event handler would get it.
    fn with_window<R>(&mut self, id: WindowId, absent: R, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        let Some(mut window) = self.windows.get_mut(&id).and_then(|w| w.window.take()) else { return absent };
        let outer = self.current_window.replace(id);
        let result = f(&mut window, self);
        self.current_window = outer;
        if let Some(slot) = self.windows.get_mut(&id) {
            slot.window = Some(window);
        }
        result
    }

    /// Takes a mounted view out of `window` for another window to adopt with its native nodes.
    pub(crate) fn lift_view(&mut self, window: WindowId, id: EntityId) -> Option<crate::tree::Lifted> {
        assert!(self.moved.insert(id), "{id:?} is rendered in two windows");
        let frame = self.frames;
        self.windows.get_mut(&window)?.tree.as_mut()?.lift(id, frame)
    }

    /// Closes the window: releases its retained tree in one last transaction.
    pub fn close_window(&mut self, id: WindowId) {
        let Some(slot) = self.windows.remove(&id) else { return };
        if let Some(tree) = slot.tree {
            tree.close(self);
        }
        self.flush_effects();
    }

    pub fn window_ids(&self) -> impl Iterator<Item = WindowId> + '_ {
        self.windows.keys().copied()
    }

    /// The window's state, for a host opening its OS window. `None` while a handler holds it.
    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.get(&id)?.window.as_ref()
    }

    /// The OS resized the window's content. Lays out in the next frame; during live resize the
    /// host calls [`Self::frame`] right after, inside the OS resize pass.
    pub fn resize_window(&mut self, id: WindowId, size: Size) {
        let Some(slot) = self.windows.get_mut(&id) else { return };
        if let Some(window) = slot.window.as_mut() {
            window.set_viewport_size(size);
        }
        if let Some(tree) = slot.tree.as_mut() {
            tree.resize(size);
        }
    }

    /// The backend's report of where the OS window controls sit. Elements padded by
    /// `titlebar_insets()` lay out again and the window's root view renders again, so a view
    /// reading [`Window::titlebar_insets`] sees the change.
    pub fn set_titlebar_insets(&mut self, id: WindowId, insets: crate::TitlebarInsets) {
        let Some(slot) = self.windows.get_mut(&id) else { return };
        if slot.window.as_ref().is_some_and(|w| w.titlebar_insets() == insets) {
            return;
        }
        if let Some(window) = slot.window.as_mut() {
            window.set_titlebar_insets(insets);
        }
        if let Some(tree) = slot.tree.as_mut() {
            tree.set_titlebar_insets(insets);
            let root = tree.root_view_id();
            self.dirty.insert(root);
        }
    }

    /// Renders the window's root view again in the next frame, as if it were notified.
    pub fn refresh_window(&mut self, id: WindowId) {
        if let Some(root) = self.windows.get(&id).and_then(|w| w.tree.as_ref()).map(|t| t.root_view_id()) {
            self.dirty.insert(root);
        }
    }

    /// The window's display clock ticked: the next frame of that window renders for `time`.
    pub fn tick_window(&mut self, id: WindowId, time: FrameTime) {
        if let Some(window) = self.windows.get_mut(&id).and_then(|w| w.window.as_mut()) {
            window.set_frame_time(time);
        }
    }

    /// The test clock: every window's clock ticks once, `interval` seconds later.
    pub fn advance_clock(&mut self, interval: f64) {
        for window in self.windows.values_mut().filter_map(|w| w.window.as_mut()) {
            window.set_frame_time(window.frame_time().advance(interval));
        }
    }

    /// The latest frame target across windows, in seconds: `cx.now()` outside a window.
    pub fn now(&self) -> f64 {
        self.windows.values().filter_map(|w| w.window.as_ref()).map(Window::now).fold(0., f64::max)
    }

    /// True when a frame would do work: a mounted view is dirty or a window needs layout.
    pub fn needs_frame(&self) -> bool {
        self.dirty.iter().any(|e| self.view_windows.contains_key(e))
            || self.windows.values().any(|w| w.window.as_ref().is_some_and(|w| w.focus_request.is_some()))
            || self.windows.values().any(|w| w.tree.as_ref().is_some_and(|t| t.needs_frame()))
    }

    /// Renders the dirty views of every window and commits one transaction per window, then
    /// drains native events. Hosts call it once per display frame while [`Self::needs_frame`].
    pub fn frame(&mut self) {
        self.flush_effects();
        self.moved.clear();
        self.frames += 1;
        let ids: Vec<WindowId> = self.windows.keys().copied().collect();
        for id in &ids {
            self.draw(*id);
        }
        for id in ids {
            let Some(mut tree) = self.windows.get_mut(&id).and_then(|w| w.tree.take()) else { continue };
            tree.release_parked(self);
            if let Some(slot) = self.windows.get_mut(&id) {
                slot.tree = Some(tree);
            }
        }
        // Whatever is still dirty is not a mounted view (a model), or was notified by a
        // render; models need no frame and renders notifying themselves would loop.
        self.dirty.retain(|e| self.view_windows.contains_key(e));
        self.drain_events();
    }

    /// Delivers queued native events to their nodes' handlers, outside every entity lease.
    /// Hosts also call it when a nested run loop (menu tracking, a modal) returns.
    pub fn drain_events(&mut self) {
        while let Some(queued) = self.events.pop() {
            let event = match queued {
                crate::events::Queued::Event(event) => event,
                crate::events::Queued::Call(window_id, call) => {
                    let Some(mut window) = self.windows.get_mut(&window_id).and_then(|slot| slot.window.take()) else {
                        continue;
                    };
                    let outer = self.current_window.replace(window_id);
                    call(&mut window, self);
                    self.current_window = outer;
                    if let Some(slot) = self.windows.get_mut(&window_id) {
                        slot.window = Some(window);
                    }
                    self.flush_effects();
                    continue;
                }
            };
            let any: &dyn Any = &*event.event;
            if let Some(resized) = any.downcast_ref::<crate::canvas::CanvasResized>() {
                self.repaint_canvas(event.node, resized.width, resized.height);
                continue;
            }
            if self.drag_event(event.node, any) {
                continue;
            }
            let type_id = any.type_id();
            let found = self.windows.iter().find_map(|(id, slot)| {
                let tree = slot.tree.as_ref()?;
                tree.contains(event.node).then(|| (*id, tree.handler(event.node, type_id)))
            });
            let Some((window_id, Some(handler))) = found else { continue };
            let slot = self.windows.get_mut(&window_id).expect("window");
            let Some(mut window) = slot.window.take() else {
                panic!("native event drained while {window_id:?} is leased");
            };
            let outer = self.current_window.replace(window_id);
            handler(any, &mut window, self);
            self.current_window = outer;
            if let Some(slot) = self.windows.get_mut(&window_id) {
                slot.window = Some(window);
                if let Some(tree) = slot.tree.as_mut() {
                    tree.reaffirm.insert(event.node);
                }
            }
            for (window, action) in std::mem::take(&mut self.deferred_actions) {
                self.dispatch_action(window, &*action);
            }
            for (window, id) in std::mem::take(&mut self.deferred_focus) {
                self.focus_id(window, &id);
            }
            self.flush_effects();
        }
    }

    /// Frames and drains until nothing is dirty and no event is queued. Panics after 100
    /// rounds, which only an update loop (a handler re-notifying itself each frame) reaches.
    pub fn run_until_idle(&mut self) {
        for _ in 0..100 {
            self.flush_effects();
            self.drain_events();
            if !self.needs_frame() && self.events.is_empty() {
                return;
            }
            self.frame();
        }
        panic!("app did not settle after 100 frames: an update loop");
    }

    pub fn events(&self) -> &EventQueue {
        &self.events
    }

    pub fn backend(&self) -> &dyn Backend {
        &*self.backend
    }

    pub fn backend_as<B: Backend>(&self) -> Option<&B> {
        let any: &dyn Any = &*self.backend;
        any.downcast_ref()
    }

    pub fn backend_as_mut<B: Backend>(&mut self) -> Option<&mut B> {
        let any: &mut dyn Any = &mut *self.backend;
        any.downcast_mut()
    }

    /// How many times the view rendered since it was created. Feeds the profiler HUD and
    /// per-view invalidation tests.
    pub fn render_count(&self, id: EntityId) -> u64 {
        self.render_counts.get(&id).copied().unwrap_or(0)
    }

    /// The view the window was opened with.
    pub fn window_root(&self, id: WindowId) -> Option<EntityId> {
        Some(self.windows.get(&id)?.tree.as_ref()?.root_view_id())
    }

    /// The native node a mounted view's output is rooted at.
    pub fn view_node(&self, id: EntityId) -> Option<NodeId> {
        let window = self.view_windows.get(&id)?;
        self.windows.get(window)?.tree.as_ref()?.view_node(id)
    }
}

/// The context an entity is updated or built with. Derefs to [`App`].
pub struct Context<'a, T> {
    app: &'a mut App,
    entity: WeakEntity<T>,
}

impl<T> Deref for Context<'_, T> {
    type Target = App;
    fn deref(&self) -> &App {
        self.app
    }
}

impl<T> DerefMut for Context<'_, T> {
    fn deref_mut(&mut self) -> &mut App {
        self.app
    }
}

impl<T: 'static> Context<'_, T> {
    pub fn entity_id(&self) -> EntityId {
        self.entity.id
    }

    pub fn entity(&self) -> Entity<T> {
        self.entity.upgrade().expect("an entity's context outlived it")
    }

    pub fn weak_entity(&self) -> WeakEntity<T> {
        self.entity.clone()
    }

    /// Re-renders this view in the next frame, and only this view.
    pub fn notify(&mut self) {
        let id = self.entity.id;
        self.app.notify(id);
    }

    /// Wraps a method-shaped handler into one an element stores. The entity is updated when
    /// the event is delivered, from the drained event queue, never from inside a native call.
    pub fn listener<E: ?Sized>(
        &self,
        f: impl Fn(&mut T, &E, &mut Window, &mut Context<T>) + 'static,
    ) -> impl Fn(&E, &mut Window, &mut App) + 'static {
        let this = self.weak_entity();
        move |event, window, cx| {
            this.update(cx, |this, cx| f(this, event, window, cx));
        }
    }

    /// Calls `f` after `entity` is notified, while both are alive.
    pub fn observe<E: 'static>(
        &mut self,
        entity: &Entity<E>,
        mut f: impl FnMut(&mut T, Entity<E>, &mut Context<T>) + 'static,
    ) -> Subscription {
        let this = self.weak_entity();
        let observed = entity.downgrade();
        let active = Rc::new(Cell::new(true));
        let callback: ObserverFn = Rc::new(RefCell::new(move |cx: &mut App| {
            let (Some(this), Some(observed)) = (this.upgrade(), observed.upgrade()) else { return false };
            this.update(cx, |this, cx| f(this, observed, cx));
            true
        }));
        self.app.observers.entry(entity.id).or_default().push(Observer { active: active.clone(), callback });
        Subscription { active, detached: false }
    }
}
