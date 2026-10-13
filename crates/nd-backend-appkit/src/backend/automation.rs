//! The semantic side of the AppKit backend: probes read live state off the NSViews, semantic
//! actions go through each control's own target/action, synthetic input is posted as NSEvents
//! into the app's own queue (no TCC, no cursor, works with the window in the background), and
//! captures render the content's layer tree.
//!
//! Coordinates are the window content's: the root node's view, flipped, below the title bar.

use nd_core::{NodeId, Point, Rect, WindowId};
use nd_ui::Keystroke;
use nd_ui::semantics::{
    ActionError, Capture, MouseButton, PointerPhase, Probe, SemanticAction, SyntheticInput, WindowState,
};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, Message};
use objc2_app_kit::{
    NSAppearanceCustomization, NSApplication, NSBitmapImageFileType, NSBitmapImageRep, NSEvent, NSEventModifierFlags,
    NSEventType, NSScreen, NSView,
};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGContext, CGImageAlphaInfo,
};
use objc2_foundation::{NSDictionary, NSPoint, NSProcessInfo, NSString};

use super::{AppKitBackend, Native, State, WindowRecord};

fn rect(r: objc2_foundation::NSRect) -> Rect {
    Rect::new(r.origin.x as f32, r.origin.y as f32, r.size.width as f32, r.size.height as f32)
}

fn not_actionable(why: impl Into<String>) -> ActionError {
    ActionError::NotActionable(why.into())
}

impl State {
    fn record_of_view(&self, view: &NSView) -> Option<&WindowRecord> {
        let window = view.window()?;
        self.windows.values().find(|r| std::ptr::eq::<objc2_app_kit::NSWindow>(&**r.window, &*window))
    }

    fn record(&self, window: WindowId) -> Result<&WindowRecord, ActionError> {
        self.windows.get(&window).ok_or(ActionError::WindowGone)
    }
}

/// A key's virtual code and the characters AppKit reports for it (`HIToolbox/Events.h`, ANSI).
fn key_code(key: &str) -> Option<(u16, String)> {
    const NAMED: [(&str, u16, u32); 15] = [
        ("enter", 0x24, 0x0d),
        ("tab", 0x30, 0x09),
        ("space", 0x31, 0x20),
        ("backspace", 0x33, 0x7f),
        ("escape", 0x35, 0x1b),
        ("delete", 0x75, 0xf728),
        ("home", 0x73, 0xf729),
        ("end", 0x77, 0xf72b),
        ("pageup", 0x74, 0xf72c),
        ("pagedown", 0x79, 0xf72d),
        ("left", 0x7b, 0xf702),
        ("right", 0x7c, 0xf703),
        ("down", 0x7d, 0xf701),
        ("up", 0x7e, 0xf700),
        ("f1", 0x7a, 0xf704),
    ];
    if let Some((_, code, ch)) = NAMED.iter().find(|(name, ..)| *name == key) {
        return Some((*code, char::from_u32(*ch)?.to_string()));
    }
    const ANSI: &str = "asdfhgzxcv\0bqweryt123465=97-80]ou[ip\0lj'k;\\,/nm.\0\0`";
    let mut chars = key.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else { return None };
    let lower = c.to_ascii_lowercase();
    let code = ANSI.chars().position(|a| a == lower && a != '\0').map_or(0, |i| i as u16);
    Some((code, c.to_string()))
}

fn modifier_flags(keystroke: &Keystroke) -> NSEventModifierFlags {
    let m = &keystroke.modifiers;
    let mut flags = NSEventModifierFlags::empty();
    for (on, flag) in [
        (m.control, NSEventModifierFlags::Control),
        (m.alt, NSEventModifierFlags::Option),
        (m.shift, NSEventModifierFlags::Shift),
        (m.platform, NSEventModifierFlags::Command),
        (m.function, NSEventModifierFlags::Function),
    ] {
        if on {
            flags |= flag;
        }
    }
    flags
}

fn now() -> f64 {
    NSProcessInfo::processInfo().systemUptime()
}

impl AppKitBackend {
    pub(super) fn probe_node(&self, node: NodeId) -> Probe {
        let Ok(state) = self.0.try_borrow() else { return Probe::default() };
        let Some(native) = state.nodes.get(&node) else { return Probe::default() };
        let mut probe = Probe::default();
        let mut content = None;
        if let Some(view) = native.view()
            && let Some(record) = state.record_of_view(view)
        {
            let root = record.root.content();
            probe.bounds = Some(rect(root.convertRect_fromView(view.bounds(), Some(view))));
            content = Some(root);
        }
        state.probe_nav(native, content.map(|c| &**c), &mut probe);
        if let Native::Button(b) = native {
            probe.enabled = Some(b.button.isEnabled());
            probe.name = Some(b.button.title().to_string());
        }
        probe
    }

    pub(super) fn window_state_of(&self, window: WindowId) -> Option<WindowState> {
        let state = self.0.try_borrow().ok()?;
        let record = state.windows.get(&window)?;
        let w = &record.window;
        let f = w.frame();
        let primary = NSScreen::screens(state.mtm).firstObject().map_or(0., |s| s.frame().size.height);
        let top = primary - (f.origin.y + f.size.height);
        Some(WindowState {
            key: w.isKeyWindow(),
            main: w.isMainWindow(),
            visible: w.isVisible(),
            frame: Some(Rect::new(f.origin.x as f32, top as f32, f.size.width as f32, f.size.height as f32)),
        })
    }

    /// The button's own action sent to its own target, as `NSAccessibilityPressAction` does:
    /// no pressed highlight and no tracking loop, about 2 ms.
    fn press(&self, node: NodeId) -> Result<(), ActionError> {
        let (button, app) = {
            let state = self.0.borrow();
            let Some(Native::Button(b)) = state.nodes.get(&node) else {
                return Err(ActionError::Unsupported(format!("click on {node:?}, which is not a button")));
            };
            if !b.button.isEnabled() {
                return Err(not_actionable(format!("{node:?} is disabled")));
            }
            (b.button.clone(), NSApplication::sharedApplication(state.mtm))
        };
        let Some(action) = button.action() else { return Err(not_actionable("button has no action")) };
        let target = button.target();
        let sender: &AnyObject = &button;
        let sent = unsafe { app.sendAction_to_from(action, target.as_deref(), Some(sender)) };
        if sent { Ok(()) } else { Err(not_actionable("no target took the button's action")) }
    }

    /// The node's center in its window's content coordinates.
    fn center(&self, node: NodeId) -> Result<(WindowId, Point), ActionError> {
        let state = self.0.borrow();
        let view = state.nodes.get(&node).and_then(Native::view).ok_or_else(|| not_actionable("no view"))?;
        let record = state.record_of_view(view).ok_or_else(|| not_actionable("not in a window"))?;
        let id = state.windows.iter().find(|(_, r)| std::ptr::eq(*r, record)).map(|(id, _)| *id).expect("window id");
        let r = record.root.content().convertRect_fromView(view.bounds(), Some(view));
        Ok((id, Point { x: (r.origin.x + r.size.width / 2.) as f32, y: (r.origin.y + r.size.height / 2.) as f32 }))
    }

    pub(super) fn perform_action(&mut self, node: NodeId, action: &SemanticAction) -> Result<(), ActionError> {
        if let Some(result) = self.perform_nav(node, action) {
            return result;
        }
        match action {
            SemanticAction::Click => self.press(node),
            SemanticAction::DoubleClick => {
                self.press(node)?;
                self.press(node)
            }
            SemanticAction::RightClick => {
                let (window, at) = self.center(node)?;
                for phase in [PointerPhase::Down, PointerPhase::Up] {
                    let input = SyntheticInput::Pointer { phase, at, button: MouseButton::Right, click_count: 1 };
                    self.post(window, &input)?;
                }
                Ok(())
            }
            SemanticAction::Hover => {
                let (window, at) = self.center(node)?;
                let input = SyntheticInput::Pointer {
                    phase: PointerPhase::Move,
                    at,
                    button: MouseButton::Left,
                    click_count: 0,
                };
                self.post(window, &input)
            }
            SemanticAction::Focus => {
                let (window, view) = {
                    let state = self.0.borrow();
                    let view =
                        state.nodes.get(&node).and_then(Native::view).ok_or_else(|| not_actionable("no view"))?;
                    let window = view.window().ok_or_else(|| not_actionable("not in a window"))?;
                    (window, view.retain())
                };
                if !view.acceptsFirstResponder() {
                    return Err(ActionError::Unsupported(format!("{node:?} takes no keyboard focus")));
                }
                if window.makeFirstResponder(Some(&view)) { Ok(()) } else { Err(not_actionable("focus refused")) }
            }
            SemanticAction::SetValue(_)
            | SemanticAction::Type(_)
            | SemanticAction::Scroll { .. }
            | SemanticAction::ScrollIntoView => {
                Err(ActionError::Unsupported(format!("{action:?}: no AppKit control takes it yet")))
            }
        }
    }

    fn mouse_event(
        &self,
        window: WindowId,
        kind: NSEventType,
        at: Point,
        click_count: u32,
    ) -> Result<Retained<NSEvent>, ActionError> {
        let state = self.0.borrow();
        let record = state.record(window)?;
        let location = record.root.content().convertPoint_toView(NSPoint::new(at.x.into(), at.y.into()), None);
        NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            kind,
            location,
            NSEventModifierFlags::empty(),
            now(),
            record.window.windowNumber(),
            None,
            0,
            click_count as isize,
            if matches!(kind, NSEventType::LeftMouseUp | NSEventType::RightMouseUp | NSEventType::MouseMoved) {
                0.
            } else {
                1.
            },
        )
        .ok_or_else(|| not_actionable("AppKit refused the mouse event"))
    }

    fn key_events(&self, window: WindowId, keystroke: &Keystroke) -> Result<Vec<Retained<NSEvent>>, ActionError> {
        let state = self.0.borrow();
        let record = state.record(window)?;
        let (code, plain) = key_code(&keystroke.key)
            .ok_or_else(|| ActionError::Unsupported(format!("key \"{}\" has no key code", keystroke.key)))?;
        let typed = keystroke
            .key_char
            .clone()
            .unwrap_or_else(|| if keystroke.modifiers.shift { plain.to_uppercase() } else { plain.clone() });
        let flags = modifier_flags(keystroke);
        [NSEventType::KeyDown, NSEventType::KeyUp]
            .into_iter()
            .map(|kind| {
                NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                    kind,
                    NSPoint::new(0., 0.),
                    flags,
                    now(),
                    record.window.windowNumber(),
                    None,
                    &NSString::from_str(&typed),
                    &NSString::from_str(&plain),
                    false,
                    code,
                )
                .ok_or_else(|| not_actionable("AppKit refused the key event"))
            })
            .collect()
    }

    /// Posts the events to the back of the app's queue in one go. A press starts AppKit's
    /// tracking loop on the main thread, which then pulls the rest of the batch from the queue.
    pub(super) fn post(&self, window: WindowId, input: &SyntheticInput) -> Result<(), ActionError> {
        let events = match input {
            SyntheticInput::Pointer { phase, at, button, click_count } => {
                let kind = match (phase, button) {
                    (PointerPhase::Move, _) => NSEventType::MouseMoved,
                    (PointerPhase::Down, MouseButton::Left) => NSEventType::LeftMouseDown,
                    (PointerPhase::Up, MouseButton::Left) => NSEventType::LeftMouseUp,
                    (PointerPhase::Down, MouseButton::Right) => NSEventType::RightMouseDown,
                    (PointerPhase::Up, MouseButton::Right) => NSEventType::RightMouseUp,
                    (PointerPhase::Down, MouseButton::Middle) => NSEventType::OtherMouseDown,
                    (PointerPhase::Up, MouseButton::Middle) => NSEventType::OtherMouseUp,
                };
                vec![self.mouse_event(window, kind, *at, *click_count)?]
            }
            SyntheticInput::Drag { from, to, steps } => {
                let steps = (*steps).max(1);
                let mut events = vec![self.mouse_event(window, NSEventType::LeftMouseDown, *from, 1)?];
                for i in 1..=steps {
                    let t = i as f32 / steps as f32;
                    let at = Point { x: from.x + (to.x - from.x) * t, y: from.y + (to.y - from.y) * t };
                    events.push(self.mouse_event(window, NSEventType::LeftMouseDragged, at, 1)?);
                }
                events.push(self.mouse_event(window, NSEventType::LeftMouseUp, *to, 1)?);
                events
            }
            SyntheticInput::Key(keystroke) => self.key_events(window, keystroke)?,
        };
        let app = NSApplication::sharedApplication(self.0.borrow().mtm);
        for event in &events {
            app.postEvent_atStart(event, false);
        }
        Ok(())
    }

    /// Renders the content root's layer tree at the backing scale. Reads Core Animation's model,
    /// not the screen: no Screen Recording grant, and the window may be covered or in the
    /// background. Window-server effects (glass, vibrancy) are absent; those need a composited
    /// capture through `ndshell`.
    pub(super) fn capture_window(&self, window: WindowId) -> Result<Capture, ActionError> {
        let state = self.0.borrow();
        let record = state.record(window)?;
        let content: &NSView = record.root.content();
        let layer = content.layer().ok_or_else(|| not_actionable("content view has no layer"))?;
        let bounds = content.bounds();
        let scale = record.window.backingScaleFactor();
        let (width, height) =
            ((bounds.size.width * scale).round() as usize, (bounds.size.height * scale).round() as usize);
        if width == 0 || height == 0 {
            return Err(not_actionable("window content has no size"));
        }
        let space = CGColorSpace::new_device_rgb().ok_or_else(|| not_actionable("no RGB colour space"))?;
        let info = CGImageAlphaInfo::PremultipliedLast.0;
        let ctx = unsafe { CGBitmapContextCreate(std::ptr::null_mut(), width, height, 8, 0, Some(&space), info) }
            .ok_or_else(|| not_actionable("CGBitmapContextCreate failed"))?;
        // The layer tree is flipped like the views; Core Graphics is not.
        CGContext::translate_ctm(Some(&ctx), 0., height as f64);
        CGContext::scale_ctm(Some(&ctx), scale, -scale);
        // The window draws its background behind the content, so the layer tree has none.
        let background = std::cell::RefCell::new(None);
        let resolve = block2::RcBlock::new(|| {
            *background.borrow_mut() = Some(record.window.backgroundColor().CGColor());
        });
        record.window.effectiveAppearance().performAsCurrentDrawingAppearance(&resolve);
        if let Some(color) = background.take() {
            CGContext::set_fill_color_with_color(Some(&ctx), Some(&color));
            CGContext::fill_rect(Some(&ctx), bounds);
        }
        layer.renderInContext(&ctx);
        let image = CGBitmapContextCreateImage(Some(&ctx)).ok_or_else(|| not_actionable("no image from context"))?;
        let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &image);
        let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }
            .ok_or_else(|| not_actionable("PNG encoding failed"))?;
        Ok(Capture { width: width as u32, height: height as u32, png: data.to_vec() })
    }
}
