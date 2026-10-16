//! The frame HUD (ARCHITECTURE.md 9.1): F12 cycles it hidden, minimal (frames per second) and
//! full (frame time, the slowest frame, frames over the display interval), read from the frames
//! nd's diagnostics record for the window. An app shows it by putting the view in its overlay
//! and answering the action at its root:
//!
//! ```ignore
//! let hud = cx.new(FrameHud::new);
//! scaffold().on_action(frame_hud_toggle(&hud)).overlay(hud.clone())
//! ```

use std::time::Duration;

use nd_diag::FrameSummary;
use nd_ui::actions::ToggleFrameHud;
use nd_ui::prelude::*;

use crate::theme::ActiveTheme;

/// How often a shown HUD reads the frames again.
const REFRESH: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HudMode {
    #[default]
    Hidden,
    Minimal,
    Full,
}

impl HudMode {
    fn next(self) -> Self {
        match self {
            HudMode::Hidden => HudMode::Minimal,
            HudMode::Minimal => HudMode::Full,
            HudMode::Full => HudMode::Hidden,
        }
    }
}

pub struct FrameHud {
    mode: HudMode,
    refreshing: bool,
}

impl FrameHud {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self { mode: HudMode::Hidden, refreshing: false }
    }

    pub fn mode(&self) -> HudMode {
        self.mode
    }

    /// Hidden, minimal, full, hidden again.
    pub fn cycle(&mut self, cx: &mut Context<Self>) {
        self.mode = self.mode.next();
        cx.notify();
    }

    fn refresh_later(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        let weak = cx.weak_entity();
        let (tx, rx) = futures_channel::oneshot::channel::<()>();
        std::thread::spawn(move || {
            std::thread::sleep(REFRESH);
            let _ = tx.send(());
        });
        crate::system::spawn(async move { rx.await.ok() }, move |_, cx| {
            weak.update(cx, |hud, cx| {
                hud.refreshing = false;
                cx.notify();
            });
        });
    }
}

/// The root's listener for [`ToggleFrameHud`] (F12 in nd's standard bindings).
pub fn frame_hud_toggle(hud: &Entity<FrameHud>) -> impl Fn(&ToggleFrameHud, &mut Window, &mut App) + 'static {
    let hud = hud.clone();
    move |_, _, cx| hud.update(cx, |hud, cx| hud.cycle(cx))
}

/// The HUD's lines for `summary` in `mode`.
pub fn hud_lines(mode: HudMode, summary: &FrameSummary) -> Vec<String> {
    match mode {
        HudMode::Hidden => Vec::new(),
        HudMode::Minimal => vec![format!("{:.0} fps", summary.fps)],
        HudMode::Full => vec![
            format!("{:.0} fps", summary.fps),
            format!("{:.1} ms a frame, {:.1} ms budget", summary.work_ms, summary.interval_ms),
            format!("{:.1} ms slowest", summary.worst_ms),
            format!("{} dropped", summary.dropped),
        ],
    }
}

impl Render for FrameHud {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.mode == HudMode::Hidden {
            return div().id("nd-frame-hud").hidden().into_any_element();
        }
        self.refresh_later(cx);
        let frames = nd_diag::global().map(|d| d.window_frames(window.id().0)).unwrap_or_default();
        let lines = hud_lines(self.mode, &FrameSummary::of(&frames));
        let theme = cx.theme().clone();
        div()
            .id("nd-frame-hud")
            .absolute()
            .top(px(60.))
            .right(px(12.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(10.))
            .py(px(6.))
            .rounded(px(8.))
            .bg(theme.surface.overlay)
            .children(
                lines
                    .into_iter()
                    .map(|line| text(line).text_size(theme.type_scale.caption).text_color(theme.text.primary)),
            )
            .into_any_element()
    }
}
