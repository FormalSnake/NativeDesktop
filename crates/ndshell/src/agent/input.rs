//! Real input from the agent: `SendInput` for mouse and keys, synthetic pointer injection for
//! pen. Coordinates are physical screen pixels (the agent is per-monitor v2 aware).

use std::time::{Duration, Instant};

use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Controls::{
    CreateSyntheticPointerDevice, DestroySyntheticPointerDevice, POINTER_FEEDBACK_DEFAULT, POINTER_TYPE_INFO,
    POINTER_TYPE_INFO_0,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, MOUSE_EVENT_FLAGS,
    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::Input::Pointer::{
    InjectSyntheticPointerInput, POINTER_FLAG_DOWN, POINTER_FLAG_INCONTACT, POINTER_FLAG_INRANGE, POINTER_FLAG_PRIMARY,
    POINTER_FLAG_UP, POINTER_FLAG_UPDATE, POINTER_FLAGS, POINTER_INFO, POINTER_PEN_INFO,
};
use windows::Win32::UI::Shell::{
    QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
};
use windows::Win32::UI::WindowsAndMessaging::WHEEL_DELTA;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, GetSystemMetrics, GetWindowThreadProcessId, PEN_MASK_PRESSURE, PT_PEN,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

/// Refuses real input over a full-screen game or presentation (ARCHITECTURE.md 10.2), unless the
/// foreground window belongs to a process the agent launched.
pub fn check_foreground(launched: &[u32]) -> Result<(), String> {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid)) };
    if launched.contains(&pid) {
        return Ok(());
    }
    match unsafe { SHQueryUserNotificationState() } {
        Ok(state) if state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_BUSY || state == QUNS_PRESENTATION_MODE => {
            Err(format!("a full-screen app is in the foreground (pid {pid}); refusing real input"))
        }
        _ => Ok(()),
    }
}

fn send(inputs: &[INPUT]) -> Result<(), String> {
    let sent = unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err(format!("SendInput posted {sent} of {} events (UIPI or a secure desktop?)", inputs.len()))
    }
}

fn mouse(flags: MOUSE_EVENT_FLAGS, dx: i32, dy: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

/// A move to the physical screen point `(x, y)`.
fn move_input(x: i32, y: i32) -> INPUT {
    let (vx, vy, vw, vh) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1),
            GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1),
        )
    };
    // Normalised 0..65535 over the virtual desktop; Windows maps back with floor(n * size / 65536),
    // so rounding up lands on the intended pixel.
    let norm = |p: i32, origin: i32, size: i32| (((p - origin) as i64 * 65536 + size as i64 - 1) / size as i64) as i32;
    mouse(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK, norm(x, vx, vw), norm(y, vy, vh))
}

pub fn cursor() -> (i32, i32) {
    let mut p = POINT::default();
    let _ = unsafe { GetCursorPos(&mut p) };
    (p.x, p.y)
}

pub fn move_to(x: i32, y: i32) -> Result<(), String> {
    send(&[move_input(x, y)])
}

pub fn button(down: bool) -> Result<(), String> {
    send(&[mouse(if down { MOUSEEVENTF_LEFTDOWN } else { MOUSEEVENTF_LEFTUP }, 0, 0)])
}

pub fn click(x: i32, y: i32) -> Result<(), String> {
    send(&[move_input(x, y), mouse(MOUSEEVENTF_LEFTDOWN, 0, 0), mouse(MOUSEEVENTF_LEFTUP, 0, 0)])
}

/// `notches` wheel notches at `(x, y)`, positive scrolling the content up as a wheel turned away.
pub fn wheel(x: i32, y: i32, notches: i32) -> Result<(), String> {
    let mut input = mouse(MOUSEEVENTF_WHEEL, 0, 0);
    input.Anonymous.mi.mouseData = (notches * WHEEL_DELTA as i32) as u32;
    send(&[move_input(x, y), input])
}

pub fn right_click(x: i32, y: i32) -> Result<(), String> {
    send(&[move_input(x, y), mouse(MOUSEEVENTF_RIGHTDOWN, 0, 0), mouse(MOUSEEVENTF_RIGHTUP, 0, 0)])
}

pub fn key(vk: u16, down: bool) -> Result<(), String> {
    let flags = if down { KEYBD_EVENT_FLAGS(0) } else { KEYEVENTF_KEYUP };
    send(&[INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    }])
}

/// Presses at `from`, moves to `to` in `steps` evenly timed moves over `duration`, releases. The
/// press and release are separate events with a pause around them, as a hand makes them, so the
/// OS enters its move or size loop before the first move arrives.
pub fn drag(from: (i32, i32), to: (i32, i32), steps: u32, duration: Duration) -> Result<(), String> {
    move_to(from.0, from.1)?;
    std::thread::sleep(Duration::from_millis(60));
    button(true)?;
    std::thread::sleep(Duration::from_millis(80));
    let start = Instant::now();
    let steps = steps.max(1);
    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        let x = from.0 + ((to.0 - from.0) as f64 * t).round() as i32;
        let y = from.1 + ((to.1 - from.1) as f64 * t).round() as i32;
        move_to(x, y)?;
        let due = start + duration.mul_f64(t);
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
    std::thread::sleep(Duration::from_millis(80));
    button(false)
}

/// A pen stroke through `points` (physical screen pixels, pressure 0 to 1), one point every
/// `interval`, through a synthetic pen device so the target sees `WM_POINTER*` with `PT_PEN`.
pub fn pen_stroke(points: &[(i32, i32, f32)], interval: Duration) -> Result<(), String> {
    if points.is_empty() {
        return Ok(());
    }
    let device = unsafe { CreateSyntheticPointerDevice(PT_PEN, 1, POINTER_FEEDBACK_DEFAULT) }
        .map_err(|e| format!("CreateSyntheticPointerDevice: {e}"))?;
    let last = points.len() - 1;
    let mut result = Ok(());
    for (i, &(x, y, pressure)) in points.iter().enumerate() {
        let phase = if i == 0 {
            POINTER_FLAG_DOWN
        } else if i == last {
            POINTER_FLAG_UP
        } else {
            POINTER_FLAG_UPDATE
        };
        let contact = if i == last { POINTER_FLAGS(0) } else { POINTER_FLAG_INCONTACT };
        let flags = POINTER_FLAGS(phase.0 | contact.0 | POINTER_FLAG_INRANGE.0 | POINTER_FLAG_PRIMARY.0);
        let info = POINTER_TYPE_INFO {
            r#type: PT_PEN,
            Anonymous: POINTER_TYPE_INFO_0 {
                penInfo: POINTER_PEN_INFO {
                    pointerInfo: POINTER_INFO {
                        pointerType: PT_PEN,
                        pointerId: 0,
                        pointerFlags: flags,
                        ptPixelLocation: POINT { x, y },
                        ..Default::default()
                    },
                    penMask: PEN_MASK_PRESSURE,
                    pressure: (pressure.clamp(0., 1.) * 1024.) as u32,
                    ..Default::default()
                },
            },
        };
        if let Err(e) = unsafe { InjectSyntheticPointerInput(device, &[info]) } {
            result = Err(format!("InjectSyntheticPointerInput at point {i}: {e}"));
            break;
        }
        std::thread::sleep(interval);
    }
    unsafe { DestroySyntheticPointerDevice(device) };
    result
}
