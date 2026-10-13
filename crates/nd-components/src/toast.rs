//! Toasts and banners.
//!
//! `toast_overlay()` shows one window's toasts, one at a time, over its content. Controlled: the
//! app owns the list, appends with [`Toast`]s and removes a toast when it hears
//! [`ToastDismissed`]. The overlay queues in list order (FIFO), a high-priority toast interrupts
//! the one on screen (which is dismissed with [`DismissReason::Interrupted`]), and a toast with a
//! timeout dismisses itself. [`ToastQueue`] holds those rules for both backends.
//!
//! `banner()` is an inline message bar with an optional action, revealed or hidden by the app.

use std::collections::HashSet;
use std::time::Duration;

use nd_core::semantic::Role;
use nd_ui::backend::Props;
use nd_ui::prelude::*;
use nd_ui::{Key, NativeElement, StyleRefinement};

/// Toast geometry, the same on every backend: the toast's design, not system metrics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToastMetrics {
    pub radius: f32,
    pub padding_x: f32,
    /// Trailing padding when an action button ends the toast: the button carries its own inset.
    pub padding_action: f32,
    pub padding_y: f32,
    pub spacing: f32,
    /// From the window's bottom edge to the toast's.
    pub bottom: f32,
    /// How far a toast slides up while it fades in.
    pub rise: f32,
    pub enter: Duration,
    pub exit: Duration,
    /// The widest a toast grows before its title truncates.
    pub max_width: f32,
}

pub const TOAST: ToastMetrics = ToastMetrics {
    radius: 14.,
    padding_x: 16.,
    padding_action: 10.,
    padding_y: 10.,
    spacing: 12.,
    bottom: 28.,
    rise: 10.,
    enter: Duration::from_millis(220),
    exit: Duration::from_millis(180),
    max_width: 480.,
};

/// The timeout a toast gets unless it names its own (libadwaita's).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ToastPriority {
    #[default]
    Normal,
    /// Interrupts the toast on screen and goes ahead of the queue.
    High,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub id: SharedString,
    pub title: SharedString,
    /// The label of the toast's one button.
    pub action: Option<SharedString>,
    /// `None` keeps the toast until the user or the app dismisses it.
    pub timeout: Option<Duration>,
    pub priority: ToastPriority,
}

/// A toast with `id` (echoed in its events) showing `title` for [`DEFAULT_TIMEOUT`].
pub fn toast(id: impl Into<SharedString>, title: impl Into<SharedString>) -> Toast {
    Toast {
        id: id.into(),
        title: title.into(),
        action: None,
        timeout: Some(DEFAULT_TIMEOUT),
        priority: ToastPriority::Normal,
    }
}

impl Toast {
    /// A button; clicking it fires [`ToastActionClicked`] and dismisses the toast.
    pub fn action(mut self, label: impl Into<SharedString>) -> Self {
        self.action = Some(label.into());
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Stays until dismissed.
    pub fn persistent(mut self) -> Self {
        self.timeout = None;
        self
    }

    pub fn high_priority(mut self) -> Self {
        self.priority = ToastPriority::High;
        self
    }
}

impl FluentBuilder for Toast {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DismissReason {
    /// Its timeout ran out.
    Timeout,
    /// The user clicked the toast.
    User,
    /// The user clicked its button ([`ToastActionClicked`] came first).
    Action,
    /// A high-priority toast took its place.
    Interrupted,
}

/// The toast left the screen, or never reached it. The app removes it from its list.
#[derive(Clone, Debug, PartialEq)]
pub struct ToastDismissed {
    pub id: SharedString,
    pub reason: DismissReason,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToastActionClicked {
    pub id: SharedString,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToastOverlayProps {
    pub toasts: Vec<Toast>,
    pub style: StyleRefinement,
}

/// Widget kind of `toast_overlay`.
pub struct ToastOverlayKind;

pub struct ToastOverlay(NativeElement<ToastOverlayProps>);

fn describe_overlay(props: &Props) -> (Role, String) {
    let name = props
        .downcast_ref::<ToastOverlayProps>()
        .and_then(|p| p.toasts.first().map(|t| t.title.to_string()))
        .unwrap_or_default();
    (Role::Status, name)
}

/// The window's toasts. Takes no space where it sits; one per window.
pub fn toast_overlay(id: impl Into<SharedString>) -> ToastOverlay {
    nd_ui::semantics::describe_widget::<ToastOverlayKind>(describe_overlay);
    let props = ToastOverlayProps { toasts: Vec::new(), style: StyleRefinement::default() };
    ToastOverlay(NativeElement::new::<ToastOverlayKind>(Key::Id(id.into()), props).absolute().size_0())
}

impl ToastOverlay {
    /// The queue, oldest first.
    pub fn toasts(mut self, toasts: impl IntoIterator<Item = Toast>) -> Self {
        self.0.props_mut().toasts = toasts.into_iter().collect();
        self
    }

    pub fn on_dismiss(self, f: impl Fn(&ToastDismissed, &mut Window, &mut App) + 'static) -> Self {
        ToastOverlay(self.0.on(f))
    }

    pub fn on_action(self, f: impl Fn(&ToastActionClicked, &mut Window, &mut App) + 'static) -> Self {
        ToastOverlay(self.0.on(f))
    }
}

impl IntoElement for ToastOverlay {
    fn into_any_element(mut self) -> AnyElement {
        let style = self.0.style().clone();
        self.0.props_mut().style = style;
        self.0.into_any_element()
    }
}

/// What a backend does after the queue changed.
#[derive(Clone, Debug, PartialEq)]
pub enum ToastChange {
    /// Show this toast (start its timeout).
    Show(Toast),
    /// The shown toast's title or button changed: redraw it in place, timer untouched.
    Refresh(Toast),
    /// Take the shown toast away.
    Hide,
    /// Report a dismissal to the app.
    Dismissed(ToastDismissed),
    Action(ToastActionClicked),
}

/// The overlay's queue rules, backend-neutral. One toast on screen; the rest wait in list order.
#[derive(Clone, Debug, Default)]
pub struct ToastQueue {
    list: Vec<Toast>,
    current: Option<Toast>,
    /// Toasts dismissed here whose removal the app has not sent yet. They never show again.
    gone: HashSet<SharedString>,
}

impl ToastQueue {
    pub fn current(&self) -> Option<&Toast> {
        self.current.as_ref()
    }

    /// The app's list. Returns what to do on screen and what to report.
    pub fn sync(&mut self, toasts: &[Toast]) -> Vec<ToastChange> {
        self.list = toasts.to_vec();
        // Ids the app dropped can come back as new toasts.
        self.gone.retain(|id| toasts.iter().any(|t| &t.id == id));
        let mut out = Vec::new();
        if let Some(current) = self.current.clone() {
            match self.list.iter().find(|t| t.id == current.id) {
                None => {
                    self.current = None;
                    out.push(ToastChange::Hide);
                }
                Some(now) if *now != current => {
                    self.current = Some(now.clone());
                    out.push(ToastChange::Refresh(now.clone()));
                }
                Some(_) => {}
            }
        }
        // A waiting high-priority toast interrupts a normal one.
        if let Some(current) = self.current.clone()
            && current.priority == ToastPriority::Normal
            && self.waiting().any(|t| t.priority == ToastPriority::High)
        {
            out.extend(self.dismiss_current(DismissReason::Interrupted));
        }
        out.extend(self.advance());
        out
    }

    /// The shown toast's timeout ran out.
    pub fn expire(&mut self, id: &str) -> Vec<ToastChange> {
        if self.current.as_ref().is_some_and(|t| t.id == id) {
            let mut out = self.dismiss_current(DismissReason::Timeout);
            out.extend(self.advance());
            return out;
        }
        Vec::new()
    }

    /// A click on the shown toast's body.
    pub fn clicked(&mut self) -> Vec<ToastChange> {
        let mut out = self.dismiss_current(DismissReason::User);
        out.extend(self.advance());
        out
    }

    /// A click on the shown toast's button.
    pub fn action(&mut self) -> Vec<ToastChange> {
        let Some(current) = self.current.clone() else { return Vec::new() };
        let mut out = vec![ToastChange::Action(ToastActionClicked { id: current.id.clone() })];
        out.extend(self.dismiss_current(DismissReason::Action));
        out.extend(self.advance());
        out
    }

    /// The overlay is going away: whatever is on screen leaves without a report.
    pub fn clear(&mut self) -> Vec<ToastChange> {
        self.current.take().map(|_| ToastChange::Hide).into_iter().collect()
    }

    fn waiting(&self) -> impl Iterator<Item = &Toast> {
        let current = self.current.as_ref().map(|t| t.id.clone());
        self.list.iter().filter(move |t| Some(&t.id) != current.as_ref() && !self.gone.contains(&t.id))
    }

    fn dismiss_current(&mut self, reason: DismissReason) -> Vec<ToastChange> {
        let Some(current) = self.current.take() else { return Vec::new() };
        self.gone.insert(current.id.clone());
        vec![ToastChange::Hide, ToastChange::Dismissed(ToastDismissed { id: current.id, reason })]
    }

    fn advance(&mut self) -> Vec<ToastChange> {
        if self.current.is_some() {
            return Vec::new();
        }
        let next =
            self.waiting().find(|t| t.priority == ToastPriority::High).or_else(|| self.waiting().next()).cloned();
        match next {
            Some(toast) => {
                self.current = Some(toast.clone());
                vec![ToastChange::Show(toast)]
            }
            None => Vec::new(),
        }
    }
}

/// Banner padding and gap: the banner's design, the same on every backend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BannerMetrics {
    pub padding_x: f32,
    pub padding_y: f32,
    pub spacing: f32,
}

pub const BANNER: BannerMetrics = BannerMetrics { padding_x: 12., padding_y: 6., spacing: 12. };

#[derive(Clone, Debug, PartialEq)]
pub struct BannerProps {
    pub title: SharedString,
    pub action: Option<SharedString>,
    pub revealed: bool,
    pub style: StyleRefinement,
}

/// The banner's button was clicked.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BannerActionClicked;

/// Widget kind of `banner`.
pub struct BannerKind;

pub struct Banner(NativeElement<BannerProps>);

fn describe_banner(props: &Props) -> (Role, String) {
    let name = props.downcast_ref::<BannerProps>().map(|p| p.title.to_string()).unwrap_or_default();
    (Role::Status, name)
}

/// A message bar across its container: `title` and an optional button. Hidden until revealed.
pub fn banner(id: impl Into<SharedString>, title: impl Into<SharedString>) -> Banner {
    nd_ui::semantics::describe_widget::<BannerKind>(describe_banner);
    let props = BannerProps { title: title.into(), action: None, revealed: true, style: StyleRefinement::default() };
    Banner(NativeElement::new::<BannerKind>(Key::Id(id.into()), props).w_full())
}

impl Banner {
    pub fn action(mut self, label: impl Into<SharedString>) -> Self {
        self.0.props_mut().action = Some(label.into());
        self
    }

    pub fn revealed(mut self, revealed: bool) -> Self {
        self.0.props_mut().revealed = revealed;
        self
    }

    pub fn on_action(self, f: impl Fn(&BannerActionClicked, &mut Window, &mut App) + 'static) -> Self {
        Banner(self.0.on(f))
    }
}

impl Styled for Banner {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl FluentBuilder for Banner {}

impl IntoElement for Banner {
    fn into_any_element(mut self) -> AnyElement {
        let style = self.0.style().clone();
        self.0.props_mut().style = style;
        self.0.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(changes: &[ToastChange]) -> Vec<&str> {
        changes
            .iter()
            .filter_map(|c| match c {
                ToastChange::Show(t) => Some(t.id.as_ref()),
                _ => None,
            })
            .collect()
    }

    fn dismissed(changes: &[ToastChange]) -> Vec<(&str, DismissReason)> {
        changes
            .iter()
            .filter_map(|c| match c {
                ToastChange::Dismissed(d) => Some((d.id.as_ref(), d.reason)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn one_toast_at_a_time_in_list_order() {
        let mut q = ToastQueue::default();
        let list = vec![toast("a", "A"), toast("b", "B")];
        assert_eq!(shown(&q.sync(&list)), ["a"]);
        assert!(q.sync(&list).is_empty(), "the same list changes nothing");
        let next = q.expire("a");
        assert_eq!(dismissed(&next), [("a", DismissReason::Timeout)]);
        assert_eq!(shown(&next), ["b"]);
        // The app removes "a" as it heard; "b" stays on screen.
        assert!(q.sync(&list[1..]).is_empty());
    }

    #[test]
    fn a_high_priority_toast_interrupts_and_jumps_the_queue() {
        let mut q = ToastQueue::default();
        q.sync(&[toast("a", "A"), toast("b", "B")]);
        let out = q.sync(&[toast("a", "A"), toast("b", "B"), toast("u", "Urgent").high_priority()]);
        assert_eq!(dismissed(&out), [("a", DismissReason::Interrupted)]);
        assert_eq!(shown(&out), ["u"]);
        let out = q.expire("u");
        assert_eq!(shown(&out), ["b"], "after the interruption the queue resumes in order");
    }

    #[test]
    fn a_high_priority_toast_does_not_interrupt_another() {
        let mut q = ToastQueue::default();
        q.sync(&[toast("u", "U").high_priority()]);
        let out = q.sync(&[toast("u", "U").high_priority(), toast("v", "V").high_priority()]);
        assert!(out.is_empty());
    }

    #[test]
    fn the_button_reports_then_dismisses() {
        let mut q = ToastQueue::default();
        q.sync(&[toast("a", "Deleted").action("Undo")]);
        let out = q.action();
        assert_eq!(out[0], ToastChange::Action(ToastActionClicked { id: "a".into() }));
        assert_eq!(dismissed(&out), [("a", DismissReason::Action)]);
        assert!(q.current().is_none());
    }

    #[test]
    fn a_toast_the_app_removes_leaves_silently() {
        let mut q = ToastQueue::default();
        q.sync(&[toast("a", "A")]);
        assert_eq!(q.sync(&[]), [ToastChange::Hide]);
    }

    #[test]
    fn a_dismissed_toast_never_shows_again_while_the_app_still_lists_it() {
        let mut q = ToastQueue::default();
        q.sync(&[toast("a", "A")]);
        q.clicked();
        assert!(q.sync(&[toast("a", "A")]).is_empty());
        q.sync(&[]);
        assert_eq!(shown(&q.sync(&[toast("a", "Again")])), ["a"], "a removed id can come back as a new toast");
    }

    #[test]
    fn an_edited_toast_refreshes_in_place() {
        let mut q = ToastQueue::default();
        q.sync(&[toast("dl", "Downloading 10%").persistent()]);
        let out = q.sync(&[toast("dl", "Downloading 60%").persistent()]);
        assert!(matches!(&out[..], [ToastChange::Refresh(t)] if t.title == "Downloading 60%"));
    }

    #[test]
    fn expiry_of_a_toast_not_on_screen_is_ignored() {
        let mut q = ToastQueue::default();
        q.sync(&[toast("a", "A"), toast("b", "B")]);
        assert!(q.expire("b").is_empty());
    }
}
