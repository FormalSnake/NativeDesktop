//! Drawn alerts and the about panel on GPUI, in libadwaita's `AdwAlertDialog` and
//! `AdwAboutDialog` look, in the window's deferred layer over the theme's scrim.
//!
//! A sheet alert covers its own window; a modal one covers every window of the app, its card on
//! the window that asked and a bare scrim on the others, and each covered window takes no input
//! but the dialog's. Every covered window's layer holds GPUI focus, so keys reach the dialog: Tab
//! and Shift-Tab move between the buttons, Return and Space press the focused one (the default
//! when it opens), Esc answers [`Alert::cancel_index`]. The answer reaches the app through
//! [`GpuiBackend::defer`], so `done` runs once, from the host's pump, never inside the GPUI
//! event that chose it.
//!
//! The view builders are pure, as in [`crate::nav`]: goldens render them as they are.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use nd_components::forms::TextStyle;
use nd_components::services::AlertCallback;
use nd_components::{AboutPanel, Alert, AlertPresentation, AlertResponse, ButtonRole};
use nd_core::{NodeId, WindowId};
use nd_gpui::gpui::{
    self, AnyElement, BoxShadow, ElementId, FocusHandle, FontWeight, Hsla, InteractiveElement, IntoElement, Keystroke,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, anchored, deferred, div, point, px,
};
use nd_theme::Theme;

use crate::backend::GpuiBackend;
use crate::forms::FormLook;
use crate::forms::glyph::{Glyph, glyph};
use crate::nav::{Marks, OnInput};
use crate::render::Build;

/// Above every overlay and panel, under only the context menu.
const DIALOG_PRIORITY: usize = usize::MAX - 1;
/// AdwAlertDialog's and AdwAboutDialog's widths, which the theme has no token for.
const ALERT_WIDTH: f32 = 372.;
const ABOUT_WIDTH: f32 = 360.;
/// The shadow under a floating dialog.
const SHADOW_RADIUS: f32 = 24.;
const SHADOW_OPACITY: f32 = 0.25;

/// What a key means to a dialog. `None` leaves it alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogKey {
    Next,
    Previous,
    Activate,
    Cancel,
}

pub fn dialog_key(keystroke: &Keystroke) -> Option<DialogKey> {
    match keystroke.key.as_str() {
        "tab" if keystroke.modifiers.shift => Some(DialogKey::Previous),
        "tab" => Some(DialogKey::Next),
        "enter" | "space" => Some(DialogKey::Activate),
        "escape" => Some(DialogKey::Cancel),
        _ => None,
    }
}

/// Input a drawn dialog turns into an answer.
#[derive(Clone, Debug, PartialEq)]
pub enum DialogInput {
    Key(DialogKey),
    /// A click on the alert button at this index of [`Alert::buttons`].
    Press(usize),
    /// The about panel's close button.
    Close,
    Open(SharedString),
    /// A press on the scrim, outside the card.
    Backdrop,
}

/// The buttons an alert shows, by their index in [`Alert::buttons`]: the platform's OK when it
/// has none.
fn buttons(alert: &Alert) -> Vec<(SharedString, ButtonRole)> {
    if alert.buttons.is_empty() {
        return vec![("OK".into(), ButtonRole::Default)];
    }
    alert.buttons.iter().map(|b| (b.label.clone(), b.role)).collect()
}

/// The button Return presses when the alert opens: the first default one, else the first.
pub fn default_button(alert: &Alert) -> usize {
    alert.buttons.iter().position(|b| b.role == ButtonRole::Default).unwrap_or(0)
}

fn shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: gpui::black().opacity(SHADOW_OPACITY),
        offset: point(px(0.), px(0.)),
        blur_radius: px(SHADOW_RADIUS),
        spread_radius: px(0.),
        inset: false,
    }]
}

/// The scrim over a whole window, holding `content` centred; it takes every press.
fn layer(
    theme: &Theme,
    viewport: gpui::Size<gpui::Pixels>,
    focus: Option<&FocusHandle>,
    on_input: Option<OnInput<DialogInput>>,
    content: Option<AnyElement>,
    marks: &Marks,
) -> AnyElement {
    marks.fill("dialog scrim", theme.scrim, &[]);
    let mut scrim = div()
        .id("nd-dialog-scrim")
        .w(viewport.width)
        .h(viewport.height)
        .flex()
        .items_center()
        .justify_center()
        .p(theme.spacing.md)
        .bg(theme.scrim)
        .occlude()
        .children(content);
    if let Some(focus) = focus {
        scrim = scrim.track_focus(focus);
    }
    if let Some(on_input) = on_input {
        let keys = on_input.clone();
        scrim = scrim
            .on_any_mouse_down(move |_, window, cx| {
                cx.stop_propagation();
                on_input(DialogInput::Backdrop, window, cx);
            })
            .on_key_down(move |event, window, cx| {
                if let Some(key) = dialog_key(&event.keystroke) {
                    cx.stop_propagation();
                    keys(DialogInput::Key(key), window, cx);
                }
            });
    }
    deferred(anchored().position(point(px(0.), px(0.))).child(scrim)).with_priority(DIALOG_PRIORITY).into_any_element()
}

/// A dialog's card: the overlay surface, rounded, over the scrim.
fn card(
    theme: &Theme,
    width: f32,
    viewport: gpui::Size<gpui::Pixels>,
    label: SharedString,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id("nd-dialog")
        .role(gpui::Role::AlertDialog)
        .aria_label(label)
        .relative()
        .flex()
        .flex_col()
        .w(px(width.min(viewport.width.as_f32() - 2. * theme.spacing.md.as_f32()).max(0.)))
        .bg(theme.surface.overlay)
        .border_1()
        .border_color(theme.border.subtle)
        .rounded(theme.radius.xl)
        .shadow(shadow())
        .text_color(theme.text.primary)
        .occlude()
        .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
}

/// A centred, wrapping line of text in `style`.
fn line(text: SharedString, style: TextStyle, color: Hsla, look: &FormLook) -> gpui::Div {
    let (size, weight, height) = look.type_of(style);
    div().w_full().text_center().text_size(size).line_height(height).font_weight(weight).text_color(color).child(text)
}

/// The alert: heading, message, and its buttons, the first trailing in a row of two, or
/// stacked first on top when there are more. `focused` is the button showing the keyboard ring.
pub fn alert_view(
    alert: &Alert,
    focused: Option<usize>,
    theme: &Theme,
    viewport: gpui::Size<gpui::Pixels>,
    focus: Option<&FocusHandle>,
    on_input: Option<OnInput<DialogInput>>,
    marks: &Marks,
) -> AnyElement {
    let look = FormLook::of(theme);
    let s = &theme.spacing;
    let surface = theme.surface.overlay;
    let under = [theme.scrim, surface];
    marks.fill("alert", surface, &[theme.scrim]);
    let (title_size, title_weight, _) = look.type_of(TextStyle::Title2);
    marks.text(&alert.title, theme.text.primary, &under, title_size, title_weight);
    let mut message = div().flex().flex_col().gap(s.md).px(s.xl).pt(s.xl).pb(s.xl).child(line(
        alert.title.clone(),
        TextStyle::Title2,
        theme.text.primary,
        &look,
    ));
    if let Some(text) = &alert.message {
        let (size, weight, _) = look.type_of(TextStyle::Body);
        marks.text(text, theme.text.primary, &under, size, weight);
        message = message.child(line(text.clone(), TextStyle::Body, theme.text.primary, &look));
    }

    let default = default_button(alert);
    let labels = buttons(alert);
    let stacked = labels.len() > 2;
    let mut row = div().flex().gap(s.md).px(s.xl).pb(s.xl);
    row = if stacked { row.flex_col() } else { row.flex_row() };
    let mut items: Vec<AnyElement> = Vec::new();
    for (i, (label, role)) in labels.iter().enumerate() {
        let (fill, text, what) = match role {
            ButtonRole::Destructive => (theme.danger, theme.text_on(theme.danger), "destructive button"),
            _ if i == default => (theme.accent, theme.text.on_accent, "suggested button"),
            // libadwaita's plain button: a tint of the card, which `surface.raised` is not on
            // a light overlay.
            _ => (theme.hover(surface), theme.text.primary, "button"),
        };
        marks.fill(what, fill, &under);
        marks.text(label, text, &[theme.scrim, surface, fill], look.text_size, FontWeight::SEMIBOLD);
        let ring = if focused == Some(i) { theme.focus_ring } else { gpui::transparent_black() };
        let hover = theme.hover(fill);
        let pressed = theme.pressed(fill);
        let mut button = div()
            .id(ElementId::NamedInteger("nd-alert-button".into(), i as u64))
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .flex()
            .items_center()
            .justify_center()
            .min_h(theme.control.large)
            .px(s.md)
            .rounded(theme.radius.lg)
            .border_2()
            .border_color(ring)
            .bg(fill)
            .text_color(text)
            .text_size(look.text_size)
            .font_weight(FontWeight::SEMIBOLD)
            .hover(move |st| st.bg(hover))
            .active(move |st| st.bg(pressed))
            .child(label.clone());
        button = if stacked { button.w_full() } else { button.flex_1().min_w_0() };
        if let Some(on_input) = on_input.clone() {
            button = button.on_click(move |_, window, cx| on_input(DialogInput::Press(i), window, cx));
        }
        items.push(button.into_any_element());
    }
    // AdwAlertDialog and NSAlert both put the first (affirmative) response trailing in a row.
    if !stacked {
        items.reverse();
    }
    let card = card(theme, ALERT_WIDTH, viewport, alert.title.clone()).child(message).child(row.children(items));
    layer(theme, viewport, focus, on_input, Some(card.into_any_element()), marks)
}

/// The about panel: name, version, credits, the website and the copyright, with a close button.
pub fn about_view(
    about: &AboutPanel,
    theme: &Theme,
    viewport: gpui::Size<gpui::Pixels>,
    focus: Option<&FocusHandle>,
    on_input: Option<OnInput<DialogInput>>,
    marks: &Marks,
) -> AnyElement {
    let look = FormLook::of(theme);
    let s = &theme.spacing;
    let surface = theme.surface.overlay;
    let under = [theme.scrim, surface];
    marks.fill("about", surface, &[theme.scrim]);
    let mut column = div().flex().flex_col().items_center().gap(s.md).px(s.xl).pt(s.xl + s.md).pb(s.xl);
    let name = about.name.clone().unwrap_or_default();
    if !name.is_empty() {
        let (size, weight, _) = look.type_of(TextStyle::Title1);
        marks.text(&name, theme.text.primary, &under, size, weight);
        column = column.child(line(name.clone(), TextStyle::Title1, theme.text.primary, &look));
    }
    if let Some(version) = &about.version {
        let text: SharedString = match &about.build {
            Some(build) => format!("{version} ({build})").into(),
            None => version.clone(),
        };
        let (bg, fg) = (theme.accent_container.bg, theme.accent_container.fg);
        marks.fill("version", bg, &under);
        marks.text(&text, fg, &[theme.scrim, surface, bg], look.text_size, FontWeight::SEMIBOLD);
        column = column.child(
            div()
                .px(s.md)
                .py(s.xs)
                .rounded(theme.radius.xl)
                .bg(bg)
                .text_color(fg)
                .text_size(look.text_size)
                .font_weight(FontWeight::SEMIBOLD)
                .child(text),
        );
    }
    if let Some(credits) = &about.credits {
        let (size, weight, _) = look.type_of(TextStyle::Body);
        marks.text(credits, theme.text.primary, &under, size, weight);
        column = column.child(line(credits.clone(), TextStyle::Body, theme.text.primary, &look));
    }
    if let Some(website) = &about.website {
        let fill = theme.hover(surface);
        marks.fill("website", fill, &under);
        marks.text(website, theme.text.primary, &[theme.scrim, surface, fill], look.text_size, FontWeight::NORMAL);
        let hover = theme.hover(fill);
        let mut link = div()
            .id("nd-about-website")
            .role(gpui::Role::Link)
            .aria_label(website.clone())
            .w_full()
            .min_h(theme.control.regular)
            .flex()
            .items_center()
            .justify_center()
            .px(s.md)
            .rounded(theme.radius.lg)
            .bg(fill)
            .text_size(look.text_size)
            .hover(move |st| st.bg(hover))
            .child(website.clone());
        if let Some(on_input) = on_input.clone() {
            let url = website.clone();
            link = link.on_click(move |_, window, cx| on_input(DialogInput::Open(url.clone()), window, cx));
        }
        column = column.child(link);
    }
    if let Some(copyright) = &about.copyright {
        let (size, weight, _) = look.type_of(TextStyle::Footnote);
        marks.text(copyright, theme.text.secondary, &under, size, weight);
        column = column.child(line(copyright.clone(), TextStyle::Footnote, theme.text.secondary, &look));
    }

    let side = theme.control.regular;
    let mut close = div()
        .id("nd-about-close")
        .role(gpui::Role::Button)
        .aria_label("Close")
        .absolute()
        .top(s.sm)
        .right(s.sm)
        .size(side)
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .hover({
            let hover = theme.hover(surface);
            move |st| st.bg(hover)
        })
        .child(glyph(Glyph::Clear, look.icon_size, theme.text.primary, look.glyph_stroke));
    marks.fill("close", theme.text.primary, &under);
    if let Some(on_input) = on_input.clone() {
        close = close.on_click(move |_, window, cx| on_input(DialogInput::Close, window, cx));
    }
    let label: SharedString = if name.is_empty() { "About".into() } else { format!("About {name}").into() };
    let card = card(theme, ABOUT_WIDTH, viewport, label).role(gpui::Role::Dialog).child(column).child(close);
    layer(theme, viewport, focus, on_input, Some(card.into_any_element()), marks)
}

/// The scrim a modal alert lays over the app's other windows.
pub fn modal_scrim_view(
    theme: &Theme,
    viewport: gpui::Size<gpui::Pixels>,
    focus: Option<&FocusHandle>,
    on_input: Option<OnInput<DialogInput>>,
    marks: &Marks,
) -> AnyElement {
    layer(theme, viewport, focus, on_input, None, marks)
}

enum Content {
    Alert {
        alert: Alert,
        done: Option<AlertCallback>,
        focused: usize,
        /// Tab moved the focus, so the ring shows.
        keyboard: bool,
    },
    About(AboutPanel),
}

struct Shown {
    owner: WindowId,
    content: Content,
}

#[derive(Default)]
pub(crate) struct DialogState {
    /// Each window's dialogs, topmost last.
    sheets: HashMap<WindowId, Vec<Shown>>,
    modal: Option<Shown>,
    /// The handle each covered window's layer tracks, and what had focus before it.
    focus: HashMap<WindowId, FocusHandle>,
    previous: HashMap<WindowId, Option<FocusHandle>>,
}

impl DialogState {
    fn covers(&self, window: WindowId) -> bool {
        self.modal.is_some() || self.sheets.get(&window).is_some_and(|s| !s.is_empty())
    }

    /// The dialog input in `window` goes to, and whether its card shows there.
    fn top_mut(&mut self, window: WindowId) -> Option<&mut Shown> {
        match &mut self.modal {
            Some(modal) => Some(modal),
            None => self.sheets.get_mut(&window)?.last_mut(),
        }
    }

    /// Takes the dialog input in `window` goes to, and whether it was the app's modal one.
    fn take_top(&mut self, window: WindowId) -> Option<(Shown, bool)> {
        if let Some(modal) = self.modal.take() {
            return Some((modal, true));
        }
        Some((self.sheets.get_mut(&window)?.pop()?, false))
    }
}

pub(crate) type Dialogs = Rc<RefCell<DialogState>>;

impl GpuiBackend {
    fn mark_dirty(&self, windows: impl IntoIterator<Item = WindowId>) {
        let mut state = self.state.borrow_mut();
        state.dirty.extend(windows);
    }

    fn all_windows(&self) -> Vec<WindowId> {
        self.state.borrow().windows.keys().copied().collect()
    }

    fn answer(&self, done: Option<AlertCallback>, index: usize) {
        if let Some(done) = done {
            self.defer(move |app| done(AlertResponse(index), app));
        }
    }

    pub(crate) fn show_alert(&self, window: WindowId, alert: Alert, done: AlertCallback) {
        if !self.state.borrow().windows.contains_key(&window) {
            let cancel = alert.cancel_index();
            return self.answer(Some(done), cancel);
        }
        let modal = alert.presentation == AlertPresentation::Modal;
        let shown = Shown {
            owner: window,
            content: Content::Alert { focused: default_button(&alert), alert, done: Some(done), keyboard: false },
        };
        let replaced = {
            let mut d = self.dialogs.borrow_mut();
            if modal {
                d.modal.replace(shown)
            } else {
                d.sheets.entry(window).or_default().push(shown);
                None
            }
        };
        // `services::alert` shows one modal alert at a time; an earlier one still up is dismissed.
        if let Some(Shown { content: Content::Alert { alert, done, .. }, .. }) = replaced {
            self.answer(done, alert.cancel_index());
        }
        let windows = if modal { self.all_windows() } else { vec![window] };
        self.mark_dirty(windows);
    }

    pub(crate) fn show_about(&self, window: Option<WindowId>, about: AboutPanel) {
        let window = window.or_else(|| {
            let state = self.state.borrow();
            state.windows.iter().filter(|(_, r)| r.root.is_some()).map(|(id, _)| *id).min()
        });
        let Some(window) = window else { return };
        {
            let mut d = self.dialogs.borrow_mut();
            let sheets = d.sheets.entry(window).or_default();
            if sheets.last().is_some_and(|s| matches!(s.content, Content::About(_))) {
                return;
            }
            sheets.push(Shown { owner: window, content: Content::About(about) });
        }
        self.mark_dirty([window]);
    }

    /// The window closed: its alerts answer as dismissed, and a modal one it asked for too.
    pub(crate) fn forget_dialogs(&self, window: WindowId) {
        let mut answers = Vec::new();
        {
            let mut d = self.dialogs.borrow_mut();
            for shown in d.sheets.remove(&window).unwrap_or_default() {
                answers.push(shown);
            }
            if d.modal.as_ref().is_some_and(|m| m.owner == window) {
                answers.extend(d.modal.take());
            }
            d.focus.remove(&window);
            d.previous.remove(&window);
        }
        for shown in answers {
            if let Content::Alert { alert, done, .. } = shown.content {
                self.answer(done, alert.cancel_index());
            }
        }
        let others = self.all_windows();
        self.mark_dirty(others);
    }

    /// Whether a dialog covers `window`: nd's gesture recognizers under it see nothing.
    pub(crate) fn dialog_covers(&self, window: WindowId) -> bool {
        self.dialogs.borrow().covers(window)
    }

    /// A covered window takes every key without the secondary modifier, so menu accelerators
    /// still work (ARCHITECTURE.md 6.2) and nothing else reaches the app under the dialog.
    pub(crate) fn dialog_claims_key(&self, keystroke: &Keystroke, window: &gpui::Window) -> bool {
        let m = &keystroke.modifiers;
        if m.platform || m.control {
            return false;
        }
        self.window_for(window.window_handle()).is_some_and(|w| self.dialog_covers(w))
    }

    /// The title of the alert, or the name in the about panel, showing on `window`.
    pub fn showing_dialog(&self, window: WindowId) -> Option<SharedString> {
        let d = self.dialogs.borrow();
        let shown = match &d.modal {
            Some(modal) if modal.owner == window => modal,
            Some(_) => return None,
            None => d.sheets.get(&window)?.last()?,
        };
        Some(match &shown.content {
            Content::Alert { alert, .. } => alert.title.clone(),
            Content::About(about) => about.name.clone().unwrap_or_else(|| "About".into()),
        })
    }

    /// Gives a covered window's layer GPUI focus, remembering what had it, and hands focus back
    /// once nothing covers the window. Before the window builds.
    pub(crate) fn prepare_dialogs(&self, window_id: WindowId, window: &mut gpui::Window, cx: &mut gpui::App) {
        let mut d = self.dialogs.borrow_mut();
        if d.covers(window_id) {
            let focus = d.focus.entry(window_id).or_insert_with(|| cx.focus_handle()).clone();
            if !focus.is_focused(window) {
                d.previous.entry(window_id).or_insert_with(|| window.focused(cx));
                drop(d);
                focus.focus(window, cx);
            }
        } else if let Some(previous) = d.previous.remove(&window_id) {
            drop(d);
            if let Some(previous) = previous {
                previous.focus(window, cx);
            }
        }
    }

    fn dialog_input(&self, window_id: WindowId, input: DialogInput, window: &mut gpui::Window, cx: &mut gpui::App) {
        enum Then {
            Nothing,
            Answer(usize),
            Close,
        }
        match &input {
            DialogInput::Backdrop => {
                self.swallow_release(window_id);
                let focus = self.dialogs.borrow().focus.get(&window_id).cloned();
                if let Some(focus) = focus {
                    focus.focus(window, cx);
                }
                return;
            }
            DialogInput::Open(url) => return cx.open_url(url),
            _ => {}
        }
        let then = {
            let mut d = self.dialogs.borrow_mut();
            let Some(top) = d.top_mut(window_id) else { return };
            match (&mut top.content, input) {
                (Content::Alert { alert, focused, keyboard, .. }, DialogInput::Key(key)) => {
                    let count = buttons(alert).len();
                    match key {
                        DialogKey::Next => {
                            *focused = (*focused + 1) % count;
                            *keyboard = true;
                            Then::Nothing
                        }
                        DialogKey::Previous => {
                            *focused = (*focused + count - 1) % count;
                            *keyboard = true;
                            Then::Nothing
                        }
                        DialogKey::Activate => Then::Answer(*focused),
                        DialogKey::Cancel => Then::Answer(alert.cancel_index()),
                    }
                }
                (Content::Alert { .. }, DialogInput::Press(i)) => Then::Answer(i),
                (Content::About(_), DialogInput::Key(DialogKey::Cancel) | DialogInput::Close) => Then::Close,
                _ => Then::Nothing,
            }
        };
        let covered = match then {
            Then::Nothing => vec![window_id],
            Then::Answer(_) | Then::Close => {
                let Some((shown, modal)) = self.dialogs.borrow_mut().take_top(window_id) else { return };
                if let (Then::Answer(i), Content::Alert { done, .. }) = (then, shown.content) {
                    self.answer(done, i);
                }
                if modal { self.all_windows() } else { vec![window_id] }
            }
        };
        self.mark_dirty(covered);
        window.refresh();
        if let Some(wake) = self.wake_fn() {
            wake(cx);
        }
    }
}

impl Build<'_> {
    /// The dialog covering this window, if any, for the root to draw above its content.
    pub(crate) fn dialog_layer(&self, root: NodeId, window: &mut gpui::Window) -> Option<AnyElement> {
        let window_id = self.window_id;
        let backend = self.backend.clone();
        let on_input: OnInput<DialogInput> =
            Rc::new(move |input, window, cx| backend.dialog_input(window_id, input, window, cx));
        let d = self.backend.dialogs.borrow();
        if !d.covers(window_id) {
            return None;
        }
        let focus = d.focus.get(&window_id);
        let theme = self.theme();
        let viewport = window.viewport_size();
        let marks = Marks::default();
        let shown = match &d.modal {
            Some(modal) => Some(modal).filter(|m| m.owner == window_id),
            None => d.sheets.get(&window_id).and_then(|s| s.last()),
        };
        let element = match shown.map(|s| &s.content) {
            Some(Content::Alert { alert, focused, keyboard, .. }) => {
                let ring = keyboard.then_some(*focused);
                alert_view(alert, ring, &theme, viewport, focus, Some(on_input), &marks)
            }
            Some(Content::About(about)) => about_view(about, &theme, viewport, focus, Some(on_input), &marks),
            None => modal_scrim_view(&theme, viewport, focus, Some(on_input), &marks),
        };
        drop(d);
        self.record_marks(root, marks.take());
        Some(element)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_the_first_default_button() {
        let confirm = Alert::confirm("Delete?", "Delete");
        assert_eq!((default_button(&confirm), confirm.cancel_index()), (0, 1));
        let save = Alert::new("Save?").cancel("Cancel").button("Save");
        assert_eq!((default_button(&save), save.cancel_index()), (1, 0));
        assert_eq!(buttons(&Alert::new("Done")).len(), 1);
    }

    #[test]
    fn keys_map_to_dialog_keys() {
        let key = |k: &str| dialog_key(&Keystroke::parse(k).expect("keystroke"));
        assert_eq!(key("tab"), Some(DialogKey::Next));
        assert_eq!(key("shift-tab"), Some(DialogKey::Previous));
        assert_eq!(key("enter"), Some(DialogKey::Activate));
        assert_eq!(key("escape"), Some(DialogKey::Cancel));
        assert_eq!(key("a"), None);
    }
}
