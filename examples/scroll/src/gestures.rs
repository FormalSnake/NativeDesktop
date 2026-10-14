//! One surface with every pointer gesture wired, compiled with the benchmark and not opened by
//! it. It shows how handlers read the event phases; the scroll arbitration it relies on is the
//! framework's.

use nd::core::Point;
use nd::prelude::*;

#[derive(Default)]
pub struct Pad {
    taps: u32,
    offset: Point,
    pressed: bool,
}

impl Pad {
    // docs:begin handlers
    fn tapped(&mut self, _: &TapEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.taps += 1;
        cx.notify();
    }

    fn reset(&mut self, _: &DoubleTapEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.taps = 0;
        self.offset = Point::default();
        cx.notify();
    }

    fn held(&mut self, event: &LongPressEvent, _: &mut Window, cx: &mut Context<Self>) {
        match event.phase {
            GesturePhase::Began => {
                self.pressed = true;
                cx.haptic(Haptic::LevelChange);
            }
            _ => self.pressed = false,
        }
        cx.notify();
    }

    fn dragged(&mut self, event: &DragEvent, _: &mut Window, cx: &mut Context<Self>) {
        match event.phase {
            GesturePhase::Began | GesturePhase::Changed => self.offset = event.translation,
            // A fling ends with a velocity in points per second; this pad just stays put.
            GesturePhase::Ended => {}
            // Another recognizer won the press: undo the preview.
            GesturePhase::Cancelled => self.offset = Point::default(),
        }
        cx.notify();
    }
    // docs:end handlers
}

impl Render for Pad {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // docs:begin wiring
        div()
            .id("pad")
            .size_full()
            .on_tap(cx.listener(Self::tapped))
            .on_double_tap(cx.listener(Self::reset))
            .on_long_press(cx.listener(Self::held))
            .on_drag(cx.listener(Self::dragged))
            .child(text(format!("{} taps, offset {:.0}, {:.0}", self.taps, self.offset.x, self.offset.y)))
        // docs:end wiring
    }
}
