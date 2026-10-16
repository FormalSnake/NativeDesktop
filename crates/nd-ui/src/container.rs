//! Native containers: a widget node with children, laid out by Taffy like a `div`, whose own
//! native object the backend picks by its widget kind (a glass view, a split view controller,
//! a toolbar). Chrome intents and `glass()` are built on it.

use std::any::TypeId;
use std::rc::Rc;

use nd_gpui::gpui::prelude::FluentBuilder;
use nd_gpui::gpui::{StyleRefinement, Styled};

use crate::app::App;
use crate::backend::{AnyProps, NodeKind, WidgetKind};
use crate::element::{
    AnyElement, DispatchProps, Element, Handler, InteractiveElement, IntoElement, Key, NodeElement, ParentElement,
};
use crate::window::Window;

/// A native container of widget kind `W` with props `P`.
pub struct ContainerElement<P> {
    kind: WidgetKind,
    key: Key,
    props: P,
    style: StyleRefinement,
    children: Vec<AnyElement>,
    handlers: Vec<(TypeId, Handler)>,
    dispatch: DispatchProps,
}

impl<P: AnyProps> ContainerElement<P> {
    pub fn new<W: 'static>(key: impl Into<Key>, props: P) -> Self {
        Self {
            kind: WidgetKind::of::<W>(),
            key: key.into(),
            props,
            style: StyleRefinement::default(),
            children: Vec::new(),
            handlers: Vec::new(),
            dispatch: DispatchProps::default(),
        }
    }

    /// A container whose props `P` describe it to the semantic tree and answer semantic
    /// actions, as [`NativeElement::described`](crate::NativeElement::described) does for leaves.
    pub fn described<W: 'static>(key: impl Into<Key>, props: P) -> Self
    where
        P: crate::semantics::WidgetSemantics,
    {
        Self { kind: WidgetKind::described::<W, P>(), ..Self::new::<W>(key, props) }
    }

    pub fn props(&self) -> &P {
        &self.props
    }

    pub fn props_mut(&mut self) -> &mut P {
        &mut self.props
    }

    pub fn key_mut(&mut self) -> &mut Key {
        &mut self.key
    }

    /// Handles events of type `E` the container emits. Replaces an earlier handler for `E`.
    pub fn on<E: 'static>(mut self, handler: impl Fn(&E, &mut Window, &mut App) + 'static) -> Self {
        let handler: Handler = Rc::new(move |event, window, cx| {
            if let Some(event) = event.downcast_ref::<E>() {
                handler(event, window, cx);
            }
        });
        self.handlers.retain(|(t, _)| *t != TypeId::of::<E>());
        self.handlers.push((TypeId::of::<E>(), handler));
        self
    }

    /// Prepends `child`, ahead of the ones added so far.
    pub fn prepend(&mut self, child: impl IntoElement) {
        self.children.insert(0, child.into_any_element());
    }
}

impl<P> Styled for ContainerElement<P> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl<P> FluentBuilder for ContainerElement<P> {}

impl<P> ParentElement for ContainerElement<P> {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl<P> InteractiveElement for ContainerElement<P> {
    fn dispatch_props(&mut self) -> &mut DispatchProps {
        &mut self.dispatch
    }
}

impl<P: AnyProps> IntoElement for ContainerElement<P> {
    fn into_any_element(self) -> AnyElement {
        AnyElement(Element::Node(Box::new(NodeElement {
            kind: NodeKind::Widget(self.kind),
            key: self.key,
            props: Rc::new(self.props),
            layout: self.style,
            measured: false,
            children: self.children,
            handlers: self.handlers,
            dispatch: self.dispatch,
            motion: None,
            paint: None,
        })))
    }
}
