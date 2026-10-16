//! Display controls: `label` (an `NSTextField` label in a system text style), `link`, and
//! `progress`/`spinner` (`NSProgressIndicator`).

use std::panic::Location;

use nd_ui::prelude::*;
use nd_ui::semantics::{Response, SemanticAction, Semantics, WidgetSemantics};
use nd_ui::{Key, LinkClicked, RichText, Role, StyleRefinement};

use super::{guard, leaf};

pub use nd_theme::TextStyle;

/// A label's colour, by meaning; each backend takes it from its system palette.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LabelTone {
    #[default]
    Primary,
    /// Supporting text (`secondaryLabelColor`).
    Secondary,
    /// An error or destructive consequence (`systemRed`).
    Danger,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LabelProps {
    pub text: SharedString,
    pub text_style: TextStyle,
    pub tone: LabelTone,
    /// The user can select and copy it.
    pub selectable: bool,
    /// Wraps onto more lines inside its width instead of truncating.
    pub wraps: bool,
    /// With `wraps`, at most this many lines; the last one ends in an ellipsis when cut.
    pub max_lines: Option<u16>,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    pub style: StyleRefinement,
    /// Runs with their own style; `text` is their concatenation.
    pub rich: Option<RichText>,
    /// A link click opens the link with the system, besides reporting [`LinkClicked`].
    pub opens_links: bool,
}

/// Widget kind of `label`.
pub struct LabelKind;

impl WidgetSemantics for LabelProps {
    fn semantics(&self) -> Semantics {
        Semantics::new(Role::Text, self.text.to_string()).enabled(self.enabled)
    }
}

pub struct Label(NativeElement<LabelProps>);

/// Text in one of the system's text styles. `text()` is the plain run; a label is the control
/// a form or window uses for titles and supporting text.
#[track_caller]
pub fn label(text: impl Into<SharedString>) -> Label {
    let props = LabelProps {
        text: text.into(),
        text_style: TextStyle::Body,
        tone: LabelTone::Primary,
        selectable: false,
        wraps: false,
        max_lines: None,
        enabled: true,
        tooltip: None,
        style: StyleRefinement::default(),
        rich: None,
        opens_links: true,
    };
    Label(NativeElement::described::<LabelKind>(Key::Site(Location::caller()), props))
}

impl Label {
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        *self.0.key_mut() = Key::Id(id.into());
        self
    }

    pub fn text_style(mut self, style: TextStyle) -> Self {
        self.0.props_mut().text_style = style;
        self
    }

    pub fn secondary(mut self) -> Self {
        self.0.props_mut().tone = LabelTone::Secondary;
        self
    }

    pub fn tone(mut self, tone: LabelTone) -> Self {
        self.0.props_mut().tone = tone;
        self
    }

    pub fn selectable(mut self) -> Self {
        self.0.props_mut().selectable = true;
        self
    }

    pub fn wraps(mut self) -> Self {
        self.0.props_mut().wraps = true;
        self
    }

    /// Wraps onto at most `lines` lines, ending the last in an ellipsis when the text is cut:
    /// a two-line message preview.
    pub fn lines(mut self, lines: u16) -> Self {
        let props = self.0.props_mut();
        props.wraps = true;
        props.max_lines = Some(lines.max(1));
        self
    }

    /// Replaces the text with runs of their own style, in the label's text style.
    pub fn rich(mut self, rich: RichText) -> Self {
        let props = self.0.props_mut();
        props.text = rich.text().clone();
        props.rich = (!rich.is_plain()).then_some(rich);
        self
    }

    /// Handles a click on a link run; the system no longer opens it.
    pub fn on_link(mut self, handler: impl Fn(&LinkClicked, &mut Window, &mut App) + 'static) -> Self {
        self.0.props_mut().opens_links = false;
        self.0 = self.0.on(handler);
        self
    }
}

leaf!(Label, LabelProps);

#[derive(Clone, Debug, PartialEq)]
pub struct LinkProps {
    pub title: SharedString,
    pub url: Option<SharedString>,
    /// Opens `url` in the default browser on activation, besides firing [`LinkActivated`].
    pub open_external: bool,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    pub style: StyleRefinement,
}

/// The user activated the link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkActivated {
    pub url: Option<SharedString>,
}

/// Widget kind of `link`.
pub struct LinkKind;

impl WidgetSemantics for LinkProps {
    fn semantics(&self) -> Semantics {
        let mut s = Semantics::new(Role::Link, self.title.to_string()).enabled(self.enabled);
        if let Some(url) = &self.url {
            s = s.value(url.to_string());
        }
        s
    }

    fn respond(&self, action: &SemanticAction) -> Option<Response> {
        if *action != SemanticAction::Click {
            return None;
        }
        Some(guard(self.enabled).unwrap_or_else(|| Ok(Box::new(LinkActivated { url: self.url.clone() }))))
    }
}

pub struct Link(NativeElement<LinkProps>);

/// A text link. With a `url` it opens in the default browser; `on_click` sees every activation.
pub fn link(id: impl Into<SharedString>, title: impl Into<SharedString>) -> Link {
    let props = LinkProps {
        title: title.into(),
        url: None,
        open_external: true,
        enabled: true,
        tooltip: None,
        style: StyleRefinement::default(),
    };
    Link(NativeElement::described::<LinkKind>(Key::Id(id.into()), props))
}

impl Link {
    pub fn url(mut self, url: impl Into<SharedString>) -> Self {
        self.0.props_mut().url = Some(url.into());
        self
    }

    /// Fires [`LinkActivated`] only; the app opens what it wants.
    pub fn keep_internal(mut self) -> Self {
        self.0.props_mut().open_external = false;
        self
    }

    pub fn on_click(self, f: impl Fn(&LinkActivated, &mut Window, &mut App) + 'static) -> Self {
        Link(self.0.on(f))
    }
}

leaf!(Link, LinkProps);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ProgressStyle {
    #[default]
    Bar,
    /// The spinning indicator; never shows a fraction.
    Spinner,
    /// A small determinate pie or ring.
    Circular,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProgressProps {
    /// Fraction done, 0 to 1; `None` is indeterminate (animated).
    pub value: Option<f32>,
    pub presentation: ProgressStyle,
    /// Accessible name ("Downloading").
    pub label: Option<SharedString>,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    pub style: StyleRefinement,
}

/// Widget kind of `progress` and `spinner`.
pub struct ProgressKind;

impl WidgetSemantics for ProgressProps {
    fn semantics(&self) -> Semantics {
        let name = self.label.clone().unwrap_or_default();
        let mut s = Semantics::new(Role::ProgressIndicator, name.to_string()).enabled(self.enabled);
        if let (Some(v), false) = (self.value, self.presentation == ProgressStyle::Spinner) {
            s = s.value(format!("{:.0}%", v.clamp(0., 1.) * 100.));
        }
        s
    }
}

pub struct Progress(NativeElement<ProgressProps>);

fn progress_with(value: Option<f32>, presentation: ProgressStyle, site: &'static Location<'static>) -> Progress {
    let props = ProgressProps {
        value,
        presentation,
        label: None,
        enabled: true,
        tooltip: None,
        style: StyleRefinement::default(),
    };
    Progress(NativeElement::described::<ProgressKind>(Key::Site(site), props))
}

/// A progress bar at `value` (0 to 1); `None` shows an indeterminate bar.
#[track_caller]
pub fn progress(value: Option<f32>) -> Progress {
    progress_with(value, ProgressStyle::Bar, Location::caller())
}

/// The spinning activity indicator.
#[track_caller]
pub fn spinner() -> Progress {
    progress_with(None, ProgressStyle::Spinner, Location::caller())
}

impl Progress {
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        *self.0.key_mut() = Key::Id(id.into());
        self
    }

    pub fn circular(mut self) -> Self {
        self.0.props_mut().presentation = ProgressStyle::Circular;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.0.props_mut().label = Some(label.into());
        self
    }
}

leaf!(Progress, ProgressProps);
