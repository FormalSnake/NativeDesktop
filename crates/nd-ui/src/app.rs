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

use nd_core::{NodeId, Size, WindowId};

use crate::backend::Backend;
use crate::element::{AnyElement, IntoElement};
use crate::events::EventQueue;
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

    pub fn has_global<G: 'static>(&self) -> bool {
        self.globals.contains_key(&TypeId::of::<G>())
    }

    /// Runs observers of notified entities and drops entities no handle refers to.
    pub fn flush_effects(&mut self) {
        if self.flushing {
            return;
        }
        self.flushing = true;
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
        let tree = Tree::new(id, root, window.viewport_size(), entity.clone().into());
        self.dirty.insert(entity.entity_id());
        self.windows.insert(id, WindowSlot { window: Some(window), tree: Some(tree) });
        WindowHandle::new(id, entity)
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

    /// True when a frame would do work: a mounted view is dirty or a window needs layout.
    pub fn needs_frame(&self) -> bool {
        self.dirty.iter().any(|e| self.view_windows.contains_key(e))
            || self.windows.values().any(|w| w.tree.as_ref().is_some_and(|t| t.needs_frame()))
    }

    /// Renders the dirty views of every window and commits one transaction per window, then
    /// drains native events. Hosts call it once per display frame while [`Self::needs_frame`].
    pub fn frame(&mut self) {
        self.flush_effects();
        let ids: Vec<WindowId> = self.windows.keys().copied().collect();
        for id in ids {
            self.draw(id);
        }
        // Whatever is still dirty is not a mounted view (a model), or was notified by a
        // render; models need no frame and renders notifying themselves would loop.
        self.dirty.retain(|e| self.view_windows.contains_key(e));
        self.drain_events();
    }

    /// Delivers queued native events to their nodes' handlers, outside every entity lease.
    /// Hosts also call it when a nested run loop (menu tracking, a modal) returns.
    pub fn drain_events(&mut self) {
        while let Some(event) = self.events.pop() {
            let any: &dyn Any = &*event.event;
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
            handler(any, &mut window, self);
            if let Some(slot) = self.windows.get_mut(&window_id) {
                slot.window = Some(window);
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
