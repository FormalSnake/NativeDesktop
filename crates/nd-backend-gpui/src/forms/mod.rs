//! Drawn forms controls (`nd_components::forms`) on GPUI, styled from [`FormLook`] for the Linux
//! look. Each control emits exactly the events its AppKit counterpart does, so the same app code
//! works on both; values stay controlled (the element shows its props, and a user change only
//! emits). What a control keeps between frames (a text field's caret, a slider mid-drag, an open
//! menu) lives in [`GpuiBackend::drawn`](crate::backend::GpuiBackend), by node, with the GPUI
//! focus handle that keys and text input arrive through.
//!
//! Focus: a click gives the control GPUI focus and reports it to nd as platform focus; nd focus
//! moving to the node (`Backend::focus`, or automation's `Focus`) gives it GPUI focus on the
//! window's next draw. Keys reach nd dispatch first (the host's keystroke interceptor); a key no
//! binding took reaches the control's key listener, and text it leaves alone reaches the focused
//! input handler, as GPUI's platforms deliver it.

mod choice;
mod display;
mod emoji;
pub(crate) mod glyph;
pub(crate) mod look;
mod pickers;
pub(crate) mod text;
pub(crate) mod toggle;
mod value;

use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use nd_components::forms::*;
use nd_core::{NodeId, Rect, WindowId};
use nd_gpui::gpui::{self, AnyElement, FocusHandle, Refineable, StyleRefinement, Styled};
use nd_ui::backend::{NodeKind, Props, WidgetKind};
use nd_ui::events::Emit;

pub use look::FormLook;

use crate::backend::{Node, PlatformFocus, Wake};
use crate::render::Build;

/// A drawn control's state between frames.
pub(crate) struct Drawn {
    pub focus: FocusHandle,
    /// The pointer gave the focus, so no ring shows until a key arrives.
    pub pointer: Rc<Cell<bool>>,
    pub state: Rc<dyn Any>,
}

fn is<W: 'static>(kind: WidgetKind) -> bool {
    kind.type_id == TypeId::of::<W>()
}

/// Whether `kind` is a forms control that takes keyboard focus in its current props.
pub(crate) fn takes_focus(kind: NodeKind, props: &Props) -> bool {
    let NodeKind::Widget(w) = kind else { return false };
    macro_rules! enabled {
        ($($kind:ty => $props:ty),* $(,)?) => {
            $(if is::<$kind>(w) {
                return props.downcast_ref::<$props>().is_some_and(|p| p.enabled);
            })*
        };
    }
    enabled!(
        TextFieldKind => TextFieldProps,
        TextAreaKind => TextAreaProps,
        ComboBoxKind => ComboBoxProps,
        CheckboxKind => CheckboxProps,
        SwitchKind => SwitchProps,
        SelectKind => SelectProps,
        SliderKind => SliderProps,
        StepperKind => StepperProps,
        DatePickerKind => DatePickerProps,
        ColorWellKind => ColorWellProps,
        LinkKind => LinkProps,
        nd_components::SeekBarKind => nd_components::SeekBarProps,
    );
    false
}

/// Where a control's user changes go: the node's event queue, the host's pump, and nd focus.
#[derive(Clone)]
pub(crate) struct Sink {
    emit: Emit,
    wake: Wake,
    pub focus: FocusHandle,
    pointer: Rc<Cell<bool>>,
    node: NodeId,
    window: WindowId,
    platform_focus: PlatformFocus,
}

impl Sink {
    /// Queues `event` for the app; handlers run on the host's next pump.
    pub fn send<E: 'static>(&self, event: E, cx: &mut gpui::App) {
        self.emit.emit(event);
        (self.wake)(cx);
    }

    pub fn send_boxed(&self, event: Box<dyn Any>, cx: &mut gpui::App) {
        self.emit.emit_boxed(event);
        (self.wake)(cx);
    }

    /// The user focused the control: GPUI focus now, nd focus on the host's next pump.
    pub fn take_focus(&self, window: &mut gpui::Window, cx: &mut gpui::App) {
        if self.focus.is_focused(window) {
            return;
        }
        self.pointer.set(true);
        window.focus(&self.focus, cx);
        self.platform_focus.borrow_mut().push((self.window, Some(self.node)));
        (self.wake)(cx);
    }

    /// A key reached the control: its focus ring shows from now on.
    pub fn keyboard(&self) {
        self.pointer.set(false);
    }
}

/// What every control builder gets.
pub(crate) struct Ctl {
    pub id: NodeId,
    pub look: FormLook,
    pub sink: Sink,
    pub focused: bool,
    /// The frame's display time in seconds.
    pub time: f64,
    pub bounds: Rc<RefCell<HashMap<NodeId, Rect>>>,
}

impl Ctl {
    pub fn focus(&self) -> &FocusHandle {
        &self.sink.focus
    }

    /// The same control around a second focusable part (the colour well's hex field).
    pub fn part(&self, focus: FocusHandle, window: &gpui::Window) -> Ctl {
        Ctl {
            id: self.id,
            look: self.look,
            focused: focus.is_focused(window),
            sink: Sink { focus, pointer: Rc::new(Cell::new(true)), ..self.sink.clone() },
            time: self.time,
            bounds: self.bounds.clone(),
        }
    }

    /// Whether the focus ring shows: focused, and not by the pointer.
    pub fn ring(&self) -> bool {
        self.focused && !self.sink.pointer.get()
    }
}

/// A div carrying the app's style over the control's defaults: `defaults` first, then `style`.
pub(crate) fn with_style<E: Styled>(mut element: E, style: &StyleRefinement) -> E {
    element.style().refine(style);
    element
}

impl Build<'_> {
    /// The control's state and focus, created on first build. A pending nd focus move to the
    /// node lands here.
    pub(crate) fn ctl<T: 'static>(
        &self,
        id: NodeId,
        enabled: bool,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
        make: impl FnOnce() -> T,
    ) -> (Ctl, Rc<RefCell<T>>) {
        let (focus, pointer, state) = {
            let mut drawn = self.backend.form_drawn.borrow_mut();
            let found = drawn.get(&id).and_then(|d| {
                Some((d.focus.clone(), d.pointer.clone(), d.state.clone().downcast::<RefCell<T>>().ok()?))
            });
            match found {
                Some(found) => found,
                None => {
                    let focus = cx.focus_handle();
                    let pointer = Rc::new(Cell::new(false));
                    let state = Rc::new(RefCell::new(make()));
                    drawn.insert(id, Drawn { focus: focus.clone(), pointer: pointer.clone(), state: state.clone() });
                    (focus, pointer, state)
                }
            }
        };
        let requested = self.backend.focus_requests.borrow().get(&self.window_id) == Some(&Some(id));
        if requested && enabled {
            self.backend.focus_requests.borrow_mut().remove(&self.window_id);
            pointer.set(false);
            window.focus(&focus, cx);
        }
        if !enabled && focus.is_focused(window) {
            window.blur(cx);
        }
        let sink = Sink {
            emit: self.backend.state.borrow().events.emitter(id),
            wake: self.wake.clone(),
            focus: focus.clone(),
            pointer,
            node: id,
            window: self.window_id,
            platform_focus: self.backend.platform_focus.clone(),
        };
        let ctl = Ctl {
            id,
            look: FormLook::of(&self.theme()),
            focused: focus.is_focused(window),
            sink,
            time: self.time,
            bounds: self.backend.bounds.clone(),
        };
        (ctl, state)
    }

    /// The drawn control for a forms kind, or `None` when `kind` is not one.
    pub(crate) fn form_control(
        &self,
        id: NodeId,
        kind: WidgetKind,
        node: &Node,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> Option<AnyElement> {
        let props = &node.props;
        let element = if is::<TextFieldKind>(kind) {
            text::text_field(self, id, props.downcast_ref()?, window, cx)
        } else if is::<TextAreaKind>(kind) {
            text::text_area(self, id, props.downcast_ref()?, window, cx)
        } else if is::<ComboBoxKind>(kind) {
            text::combo_box(self, id, props.downcast_ref()?, window, cx)
        } else if is::<CheckboxKind>(kind) {
            toggle::checkbox(self, id, props.downcast_ref()?, window, cx)
        } else if is::<SwitchKind>(kind) {
            toggle::switch(self, id, props.downcast_ref()?, window, cx)
        } else if is::<SelectKind>(kind) {
            choice::select(self, id, props.downcast_ref()?, window, cx)
        } else if is::<SliderKind>(kind) {
            value::slider(self, id, props.downcast_ref()?, window, cx)
        } else if is::<StepperKind>(kind) {
            value::stepper(self, id, props.downcast_ref()?, window, cx)
        } else if is::<DatePickerKind>(kind) {
            pickers::date_picker(self, id, props.downcast_ref()?, window, cx)
        } else if is::<ColorWellKind>(kind) {
            pickers::color_well(self, id, props.downcast_ref()?, window, cx)
        } else if is::<ProgressKind>(kind) {
            display::progress(self, id, props.downcast_ref()?, window)
        } else if is::<LabelKind>(kind) {
            display::label(self, id, props.downcast_ref()?)
        } else if is::<LinkKind>(kind) {
            display::link(self, id, props.downcast_ref()?, window, cx)
        } else {
            return None;
        };
        Some(element)
    }
}
