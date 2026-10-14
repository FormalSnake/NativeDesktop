//! `terminal(id, &session)`: the element that shows a session, and the events it reports.

use nd_ui::actions::{Cancel, FocusNext, FocusPrevious};
use nd_ui::prelude::*;
use nd_ui::{App, InteractiveElement, NativeElement, StyleRefinement, Window};

use crate::input::{Key, KeyInput, Mods};

use crate::session::TerminalSession;
use crate::theme::TerminalColors;
use crate::{ExitStatus, Link};

/// The widget kind backends register their terminal presenter for.
pub struct TerminalWidget;

/// What a `terminal()` element shows and how.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalProps {
    pub session: TerminalSession,
    /// Points; the platform's monospace size when `None`.
    pub font_size: Option<f32>,
    /// A family name; the theme's monospace font when `None` or not installed.
    pub font_family: Option<SharedString>,
    pub colors: TerminalColors,
    /// Space between the grid and the element's edges, in points.
    pub padding: f32,
    /// The element's layout style, for backends that lay widgets out themselves.
    pub style: StyleRefinement,
}

/// The program set the title (OSC 0 or 2).
#[derive(Clone, Debug, PartialEq)]
pub struct TitleChanged {
    pub title: String,
}

/// The program reported its working directory (OSC 7).
#[derive(Clone, Debug, PartialEq)]
pub struct CwdChanged {
    pub cwd: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bell;

#[derive(Clone, Debug, PartialEq)]
pub struct Exited {
    pub status: ExitStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectionChanged {
    pub has_selection: bool,
}

/// A link was opened with the platform modifier (Cmd on macOS, Ctrl elsewhere) and a click.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkActivated {
    pub link: Link,
}

/// A desktop notification the program asked for (OSC 9 or 777).
#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    pub title: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FocusChanged {
    pub focused: bool,
}

/// Shows `session`. Its size follows the element's: the grid takes as many cells as fit.
pub fn terminal(id: impl Into<nd_ui::Key>, session: &TerminalSession) -> Terminal {
    Terminal(NativeElement::new::<TerminalWidget>(
        id,
        TerminalProps {
            session: session.clone(),
            font_size: None,
            font_family: None,
            colors: TerminalColors::default(),
            padding: 6.,
            style: StyleRefinement::default(),
        },
    ))
}

pub struct Terminal(NativeElement<TerminalProps>);

impl Terminal {
    pub fn font_size(mut self, size: f32) -> Self {
        self.0.props_mut().font_size = Some(size);
        self
    }

    pub fn font_family(mut self, family: impl Into<SharedString>) -> Self {
        self.0.props_mut().font_family = Some(family.into());
        self
    }

    /// Colours over the theme's: any left `None` stays theme-derived.
    pub fn colors(mut self, colors: TerminalColors) -> Self {
        self.0.props_mut().colors = colors;
        self
    }

    pub fn padding(mut self, points: f32) -> Self {
        self.0.props_mut().padding = points;
        self
    }

    pub fn on_title_changed(self, f: impl Fn(&TitleChanged, &mut Window, &mut App) + 'static) -> Self {
        Self(self.0.on(f))
    }

    pub fn on_cwd_changed(self, f: impl Fn(&CwdChanged, &mut Window, &mut App) + 'static) -> Self {
        Self(self.0.on(f))
    }

    pub fn on_bell(self, f: impl Fn(&Bell, &mut Window, &mut App) + 'static) -> Self {
        Self(self.0.on(f))
    }

    pub fn on_exit(self, f: impl Fn(&Exited, &mut Window, &mut App) + 'static) -> Self {
        Self(self.0.on(f))
    }

    pub fn on_selection_changed(self, f: impl Fn(&SelectionChanged, &mut Window, &mut App) + 'static) -> Self {
        Self(self.0.on(f))
    }

    pub fn on_link(self, f: impl Fn(&LinkActivated, &mut Window, &mut App) + 'static) -> Self {
        Self(self.0.on(f))
    }

    pub fn on_notification(self, f: impl Fn(&Notification, &mut Window, &mut App) + 'static) -> Self {
        Self(self.0.on(f))
    }

    pub fn on_focus_changed(self, f: impl Fn(&FocusChanged, &mut Window, &mut App) + 'static) -> Self {
        Self(self.0.on(f))
    }
}

impl Styled for Terminal {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl FluentBuilder for Terminal {}

impl InteractiveElement for Terminal {
    fn dispatch_props(&mut self) -> &mut nd_ui::DispatchProps {
        self.0.dispatch_props()
    }
}

impl IntoElement for Terminal {
    fn into_any_element(mut self) -> AnyElement {
        crate::register_presenters();
        let style = self.0.style().clone();
        self.0.props_mut().style = style;
        // Focused, the terminal keeps the keys nd binds for itself: Tab and Shift-Tab traverse
        // focus, Return activates a default button and Escape dismisses a scope everywhere else.
        let key = |key: Key, mods: Mods| {
            let session = self.0.props().session.clone();
            move || {
                session.key(&KeyInput::press(key, mods));
            }
        };
        let (tab, back_tab, escape) =
            (key(Key::Tab, Mods::empty()), key(Key::Tab, Mods::SHIFT), key(Key::Escape, Mods::empty()));
        self.0
            .focusable()
            .takes_return()
            .on_action(move |_: &FocusNext, _, _| tab())
            .on_action(move |_: &FocusPrevious, _, _| back_tab())
            .on_action(move |_: &Cancel, _, _| escape())
            .into_any_element()
    }
}

/// Turns session events into the element's events on `emit`.
pub(crate) fn emit_session_events(session: &TerminalSession, emit: &nd_ui::events::Emit) -> bool {
    let events = session.take_events();
    let any = !events.is_empty();
    for event in events {
        use crate::SessionEvent as S;
        match event {
            S::TitleChanged(title) => emit.emit(TitleChanged { title }),
            S::CwdChanged(cwd) => emit.emit(CwdChanged { cwd }),
            S::Bell => emit.emit(Bell),
            S::Exited(status) => emit.emit(Exited { status }),
            S::SelectionChanged(has_selection) => emit.emit(SelectionChanged { has_selection }),
            S::Notification { title, body } => emit.emit(Notification { title, body }),
        }
    }
    any
}
