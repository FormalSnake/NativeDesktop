//! The semantic tree on the `nd-ui` side (ARCHITECTURE.md 10.1): what each node kind means to a
//! user, what a backend may add from the native object ([`Probe`]), and the semantic actions and
//! synthetic input automation drives through the backend.
//!
//! The one entry point for readers (automation, diagnostics, MCP) is
//! [`App::semantic_tree`](crate::App::semantic_tree); actions go through
//! [`App::perform`](crate::App::perform) and [`App::synthesize`](crate::App::synthesize).

use std::fmt;

use nd_core::semantic::Role;
use nd_core::{Point, Rect};
use nd_gpui::gpui::Keystroke;

use crate::backend::{NodeKind, Props};
use crate::element::{ButtonProps, TextProps};
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

/// What the retained tree says about a node from its kind and props alone.
pub(crate) struct Described {
    pub role: Role,
    pub kind: String,
    pub name: String,
}

/// The short type name of a widget kind: `nd_ui::element::Button` is `Button`.
fn short_name(name: &str) -> &str {
    let base = name.split('<').next().unwrap_or(name);
    base.rsplit("::").next().unwrap_or(base)
}

pub(crate) fn describe(kind: NodeKind, props: &Props) -> Described {
    let (role, kind_name, name) = match kind {
        NodeKind::Root => (Role::Window, "root".to_owned(), String::new()),
        NodeKind::View => (Role::Group, "view".to_owned(), String::new()),
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
        NodeKind::Widget(w) => (Role::Generic, short_name(w.name).to_owned(), String::new()),
    };
    Described { role, kind: kind_name, name }
}
