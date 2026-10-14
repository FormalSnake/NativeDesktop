//! Two windows that animate every frame, each showing the rate its own display clock runs at
//! (ARCHITECTURE.md 5.3). Put them on displays with different refresh rates and each keeps its
//! own; `ND_PACING=1` prints `nd-pacing:` lines with the measured rate per window.

use nd::prelude::*;

pub struct Clock {
    name: &'static str,
}

impl Render for Clock {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // docs:begin clock
        // Rendering again next frame keeps the window animating at its display's rate.
        cx.notify();
        let time = window.frame_time();
        let hz = if time.interval() > 0. { 1. / time.interval() } else { 0. };
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_1()
            .child(text(self.name).text_size(px(20.)))
            .child(text(format!("frame {}", time.frame)).tabular_nums())
            .child(text(format!("{hz:.0} Hz display")).tabular_nums())
        // docs:end clock
    }
}

pub fn app() -> Application {
    Application::new(nd::manifest!())
}

pub fn open(cx: &mut App, name: &'static str) -> WindowHandle<Clock> {
    cx.open_window(WindowOptions::main(name).title(name).size(size(px(360.), px(200.))), |_, cx| {
        cx.new(|_| Clock { name })
    })
}
