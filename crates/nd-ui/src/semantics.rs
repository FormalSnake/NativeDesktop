//! The semantic tree on the `nd-ui` side (ARCHITECTURE.md 10.1): what each node kind means to a
//! user, what a backend may add from the native object ([`Probe`]), and the semantic actions and
//! synthetic input automation drives through the backend.
//!
//! The one entry point for readers (automation, diagnostics, MCP) is
//! [`App::semantic_tree`](crate::App::semantic_tree); actions go through
//! [`App::perform`](crate::App::perform) and [`App::synthesize`](crate::App::synthesize).

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;

use nd_core::semantic::Role;
use nd_core::{NodeId, Point, Rect};
use nd_gpui::gpui::{Hsla, Keystroke};

use crate::backend::{NodeKind, Props};
use crate::element::{ButtonProps, Scope, TextProps};
use crate::{Button, WindowControls};

/// What a backend reads from a node's native object, overriding what the retained tree says.
/// `None` fields keep the retained tree's answer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Probe {
    /// In the window content's coordinates, top-left origin.
    pub bounds: Option<Rect>,
    pub name: Option<String>,
    pub value: Option<String>,
    pub enabled: Option<bool>,
    pub checked: Option<bool>,
    pub selected: Option<bool>,
    pub expanded: Option<bool>,
    pub placeholder: Option<String>,
}

/// A window as the OS has it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WindowState {
    pub key: bool,
    pub main: bool,
    pub visible: bool,
    /// Global screen points, top-left origin.
    pub frame: Option<Rect>,
}

/// An action on one node, carried out the way the platform's accessibility action would: through
/// the control's own target/action, never by moving the cursor.
#[derive(Clone, Debug, PartialEq)]
pub enum SemanticAction {
    Click,
    DoubleClick,
    RightClick,
    Hover,
    Focus,
    /// Replaces the control's value.
    SetValue(String),
    /// Appends text, as typing would.
    Type(String),
    Scroll {
        dx: f32,
        dy: f32,
    },
    ScrollIntoView,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerPhase {
    Down,
    Up,
    Move,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
}

/// Input events posted into the app's own event queue (NSEvents on AppKit): no cursor moves, no
/// focus stolen from other apps, so these need no machine lock.
#[derive(Clone, Debug, PartialEq)]
pub enum SyntheticInput {
    /// `at` is in the window content's coordinates.
    Pointer {
        phase: PointerPhase,
        at: Point,
        button: MouseButton,
        click_count: u32,
    },
    /// Press, `steps` drags and release, posted as one batch: AppKit tracking loops block the
    /// main thread mid-gesture, so a gesture split across calls would deadlock.
    Drag {
        from: Point,
        to: Point,
        steps: u32,
    },
    Key(Keystroke),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionError {
    /// The node is gone or cannot take the action now; the reason says which.
    NotActionable(String),
    /// The backend does not implement this action for this kind of node.
    Unsupported(String),
    /// The backend cannot synthesize input events.
    InputUnsupported,
    /// The window is closed.
    WindowGone,
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionError::NotActionable(why) => write!(f, "not actionable: {why}"),
            ActionError::Unsupported(what) => write!(f, "unsupported: {what}"),
            ActionError::InputUnsupported => f.write_str("input synthesis unsupported on this backend"),
            ActionError::WindowGone => f.write_str("window closed"),
        }
    }
}

impl std::error::Error for ActionError {}

/// A PNG of a window's content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
}

/// What a widget's props say about it: role, name and the state a control holds. A backend's
/// [`Probe`] of the live native object wins over each field it sets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Semantics {
    pub role: Option<Role>,
    pub name: String,
    pub value: Option<String>,
    pub placeholder: Option<String>,
    pub enabled: Option<bool>,
    pub checked: Option<bool>,
    pub selected: Option<bool>,
    pub expanded: Option<bool>,
    /// Parts of the one native control a user perceives as nodes of their own (a palette's
    /// search box). They appear as its children; actions on one go to the control.
    pub parts: Vec<SemanticPart>,
}

/// A node inside a widget, without a retained node of its own.
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticPart {
    /// Joined to the widget's test id with a dot (`palette.search`).
    pub key: &'static str,
    pub role: Role,
    pub name: String,
    pub value: Option<String>,
    pub placeholder: Option<String>,
    /// Has keyboard focus while the widget does.
    pub focused: bool,
}

const PART_BIT: u64 = 1 << 63;

/// The id of the `index`th part of `owner` in the semantic tree.
pub fn part_id(owner: NodeId, index: usize) -> NodeId {
    NodeId(PART_BIT | (owner.0 << 8) | index as u64)
}

/// The widget a part id belongs to; `None` for an ordinary node.
pub fn part_owner(id: NodeId) -> Option<NodeId> {
    (id.0 & PART_BIT != 0).then_some(NodeId((id.0 & !PART_BIT) >> 8))
}

impl Semantics {
    pub fn new(role: Role, name: impl Into<String>) -> Self {
        Self { role: Some(role), name: name.into(), ..Self::default() }
    }

    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    pub fn placeholder(mut self, placeholder: Option<impl Into<String>>) -> Self {
        self.placeholder = placeholder.map(Into::into);
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = Some(enabled);
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }
}

/// A control's answer to a semantic action: the event its native control would emit, or why
/// it cannot take the action.
pub type Response = Result<Box<dyn Any>, ActionError>;

/// Props of a widget that describe it to the semantic tree and automation. Kinds built with
/// [`WidgetKind::described`](crate::backend::WidgetKind::described) use it.
pub trait WidgetSemantics: crate::backend::AnyProps + Sized {
    fn semantics(&self) -> Semantics;

    /// The event the control emits when automation performs `action` on it, as the user doing
    /// the same would (`SetValue("0.5")` on a slider is a `SliderChanged`). `None` when the
    /// control does not take the action. Backends that draw the control themselves emit it;
    /// AppKit drives the real control instead.
    fn respond(&self, _action: &SemanticAction) -> Option<Response> {
        None
    }
}

/// One colour a backend painted at rest, read back by the appearance audit.
#[derive(Clone, Debug)]
pub struct Paint {
    /// The node it belongs to, when the painted object maps to one.
    pub node: Option<NodeId>,
    pub kind: PaintKind,
    /// As handed to the renderer, alpha included.
    pub color: Hsla,
    /// What it is painted over, composited down to opaque.
    pub background: Hsla,
    /// The subtree scopes in effect, outermost first, so a reader can resolve the theme the
    /// colour should come from.
    pub scopes: Vec<Scope>,
    /// The text, or what the fill is for.
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaintKind {
    /// A text run at `size` points and CSS `weight`.
    Text {
        size: f32,
        weight: f32,
    },
    Fill,
}

/// What the retained tree says about a node from its kind and props alone.
pub(crate) struct Described {
    pub role: Role,
    pub kind: String,
    pub name: String,
    pub extra: Semantics,
}

/// The short type name of a widget kind: `nd_ui::element::Button` is `Button`.
fn short_name(name: &str) -> &str {
    let base = name.split('<').next().unwrap_or(name);
    base.rsplit("::").next().unwrap_or(base)
}

pub(crate) fn describe(kind: NodeKind, props: &Props) -> Described {
    if let NodeKind::Widget(w) = kind
        && let Some(extra) = w.semantics(&**props)
    {
        return Described {
            role: extra.role.unwrap_or(Role::Generic),
            kind: short_name(w.name).to_owned(),
            name: extra.name.clone(),
            extra,
        };
    }
    if let NodeKind::View = kind
        && let Some(extra) = props.downcast_ref::<crate::DivProps>().and_then(|p| p.scope.as_ref()?.get::<Semantics>())
    {
        return Described {
            role: extra.role.unwrap_or(Role::Group),
            kind: "view".to_owned(),
            name: extra.name.clone(),
            extra: extra.clone(),
        };
    }
    let (role, kind_name, name) = match kind {
        NodeKind::Root => (Role::Window, "root".to_owned(), String::new()),
        NodeKind::View => match props.downcast_ref::<crate::DivProps>().and_then(|p| p.role.clone()) {
            Some((role, name)) => (role, "view".to_owned(), name.to_string()),
            None => (Role::Group, "view".to_owned(), String::new()),
        },
        NodeKind::Text => {
            let text = props.downcast_ref::<TextProps>().map(|p| p.text.to_string()).unwrap_or_default();
            (Role::Text, "text".to_owned(), text)
        }
        NodeKind::Canvas => (Role::Canvas, "canvas".to_owned(), String::new()),
        NodeKind::Image => (Role::Image, "image".to_owned(), String::new()),
        NodeKind::Widget(w) if w.type_id == std::any::TypeId::of::<Button>() => {
            let label = props.downcast_ref::<ButtonProps>().map(|p| p.label.to_string()).unwrap_or_default();
            (Role::Button, "Button".to_owned(), label)
        }
        NodeKind::Widget(w) if w.type_id == std::any::TypeId::of::<WindowControls>() => {
            (Role::WindowControls, "WindowControls".to_owned(), String::new())
        }
        NodeKind::Widget(w) => {
            let (role, name) = WIDGETS
                .with(|m| m.borrow().get(&w.type_id).map(|d| d(props)))
                .unwrap_or((Role::Generic, String::new()));
            (role, short_name(w.name).to_owned(), name)
        }
    };
    let mut extra = Semantics::default();
    if let Some(button) = props.downcast_ref::<ButtonProps>() {
        extra.enabled = Some(button.enabled);
    }
    Described { role, kind: kind_name, name, extra }
}

/// A widget kind's role and accessible name, read from its props.
pub type DescribeWidget = fn(&Props) -> (Role, String);

thread_local! {
    static WIDGETS: RefCell<HashMap<TypeId, DescribeWidget>> = RefCell::new(HashMap::new());
}

/// Gives widget kind `W` a role and name in the semantic tree. Widgets defined outside `nd-ui`
/// call it from their constructor; a kind nobody describes is `Role::Generic`.
pub fn describe_widget<W: 'static>(describe: DescribeWidget) {
    WIDGETS.with(|m| {
        m.borrow_mut().entry(TypeId::of::<W>()).or_insert(describe);
    });
}
