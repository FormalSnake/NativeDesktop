//! Synthetic input as GPUI platform events, and PNG encoding for captures.

use nd_gpui::gpui::{
    self, KeyDownEvent, KeyUpEvent, Modifiers, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, point, px,
};
use nd_ui::semantics::{MouseButton, PointerPhase, SyntheticInput};

fn button(b: MouseButton) -> gpui::MouseButton {
    match b {
        MouseButton::Left => gpui::MouseButton::Left,
        MouseButton::Right => gpui::MouseButton::Right,
        MouseButton::Middle => gpui::MouseButton::Middle,
    }
}

fn at(p: nd_core::Point) -> gpui::Point<gpui::Pixels> {
    point(px(p.x), px(p.y))
}

/// The events one [`SyntheticInput`] stands for, in dispatch order.
pub(crate) fn platform_inputs(input: &SyntheticInput) -> Vec<PlatformInput> {
    let modifiers = Modifiers::default();
    match input {
        SyntheticInput::Pointer { phase, at: p, button: b, click_count } => {
            let position = at(*p);
            let click_count = *click_count as usize;
            vec![match phase {
                PointerPhase::Move => {
                    PlatformInput::MouseMove(MouseMoveEvent { position, pressed_button: None, modifiers })
                }
                PointerPhase::Down => PlatformInput::MouseDown(MouseDownEvent {
                    button: button(*b),
                    position,
                    modifiers,
                    click_count,
                    first_mouse: false,
                }),
                PointerPhase::Up => {
                    PlatformInput::MouseUp(MouseUpEvent { button: button(*b), position, modifiers, click_count })
                }
            }]
        }
        SyntheticInput::Drag { from, to, steps } => {
            let steps = (*steps).max(1);
            let mut events = vec![PlatformInput::MouseDown(MouseDownEvent {
                button: gpui::MouseButton::Left,
                position: at(*from),
                modifiers,
                click_count: 1,
                first_mouse: false,
            })];
            for i in 1..=steps {
                let t = i as f32 / steps as f32;
                let p = nd_core::Point { x: from.x + (to.x - from.x) * t, y: from.y + (to.y - from.y) * t };
                events.push(PlatformInput::MouseMove(MouseMoveEvent {
                    position: at(p),
                    pressed_button: Some(gpui::MouseButton::Left),
                    modifiers,
                }));
            }
            events.push(PlatformInput::MouseUp(MouseUpEvent {
                button: gpui::MouseButton::Left,
                position: at(*to),
                modifiers,
                click_count: 1,
            }));
            events
        }
        SyntheticInput::Key(keystroke) => vec![
            PlatformInput::KeyDown(KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            }),
            PlatformInput::KeyUp(KeyUpEvent { keystroke: keystroke.clone() }),
        ],
    }
}

#[cfg(feature = "capture")]
pub(crate) fn png(image: &image::RgbaImage) -> Result<nd_ui::semantics::Capture, image::ImageError> {
    let mut png = Vec::new();
    image.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
    Ok(nd_ui::semantics::Capture { width: image.width(), height: image.height(), png })
}
