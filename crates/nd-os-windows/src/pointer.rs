//! Pen and touch from `WM_POINTER*` (`docs/research/windows-packaging-updates.md` section 2).
//! GPUI handles only the mouse messages Windows promotes pointers to, which carry no pressure,
//! tilt or eraser. The window procedure reads the pointer here first, then lets the message go
//! on, so the promoted mouse input still reaches the backend.

/// `POINTER_INPUT_TYPE` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerKind {
    Mouse,
    Touch,
    Pen,
    Touchpad,
}

impl PointerKind {
    pub fn from_raw(raw: i32) -> Option<Self> {
        match raw {
            2 => Some(Self::Touch),
            3 => Some(Self::Pen),
            4 => Some(Self::Mouse),
            5 => Some(Self::Touchpad),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerPhase {
    Down,
    Update,
    Up,
}

/// What a pen reports beyond its position.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PenSample {
    /// 0 to 1; `None` when the digitizer has no pressure axis.
    pub pressure: Option<f32>,
    /// Degrees, -90 to 90.
    pub tilt_x: Option<f32>,
    pub tilt_y: Option<f32>,
    /// Degrees clockwise, 0 to 359.
    pub rotation: Option<f32>,
    pub barrel: bool,
    pub inverted: bool,
    pub eraser: bool,
}

const PEN_FLAG_BARREL: u32 = 0x1;
const PEN_FLAG_INVERTED: u32 = 0x2;
const PEN_FLAG_ERASER: u32 = 0x4;
const PEN_MASK_PRESSURE: u32 = 0x1;
const PEN_MASK_ROTATION: u32 = 0x2;
const PEN_MASK_TILT_X: u32 = 0x4;
const PEN_MASK_TILT_Y: u32 = 0x8;
/// `POINTER_PEN_INFO.pressure` runs 0 to 1024.
const PEN_PRESSURE_MAX: f32 = 1024.;

impl PenSample {
    /// Decodes the fields of a `POINTER_PEN_INFO`; axes its `penMask` leaves out are `None`.
    pub fn decode(flags: u32, mask: u32, pressure: u32, rotation: u32, tilt_x: i32, tilt_y: i32) -> Self {
        let has = |bit: u32| mask & bit != 0;
        Self {
            pressure: has(PEN_MASK_PRESSURE).then(|| (pressure as f32 / PEN_PRESSURE_MAX).clamp(0., 1.)),
            rotation: has(PEN_MASK_ROTATION).then_some(rotation as f32),
            tilt_x: has(PEN_MASK_TILT_X).then_some(tilt_x as f32),
            tilt_y: has(PEN_MASK_TILT_Y).then_some(tilt_y as f32),
            barrel: flags & PEN_FLAG_BARREL != 0,
            inverted: flags & PEN_FLAG_INVERTED != 0,
            eraser: flags & PEN_FLAG_ERASER != 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerEvent {
    pub id: u32,
    pub kind: PointerKind,
    pub phase: PointerPhase,
    /// Logical pixels from the client area's top-left.
    pub x: f32,
    pub y: f32,
    pub in_contact: bool,
    /// Set for pens.
    pub pen: Option<PenSample>,
}

#[cfg(windows)]
pub(crate) fn read(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    scale: f32,
) -> Option<PointerEvent> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::ScreenToClient;
    use windows::Win32::UI::Input::Pointer::{
        GetPointerInfo, GetPointerPenInfo, POINTER_FLAG_INCONTACT, POINTER_INFO, POINTER_PEN_INFO,
    };
    use windows::Win32::UI::WindowsAndMessaging::{PT_PEN, WM_POINTERDOWN, WM_POINTERUP, WM_POINTERUPDATE};

    let phase = match msg {
        WM_POINTERDOWN => PointerPhase::Down,
        WM_POINTERUPDATE => PointerPhase::Update,
        WM_POINTERUP => PointerPhase::Up,
        _ => return None,
    };
    // GET_POINTERID_WPARAM: the low word.
    let id = (wparam.0 & 0xffff) as u32;
    let mut info = POINTER_INFO::default();
    unsafe { GetPointerInfo(id, &mut info) }.ok()?;
    let kind = PointerKind::from_raw(info.pointerType.0)?;
    let pen = if info.pointerType == PT_PEN {
        let mut pen = POINTER_PEN_INFO::default();
        unsafe { GetPointerPenInfo(id, &mut pen) }.ok()?;
        Some(PenSample::decode(pen.penFlags, pen.penMask, pen.pressure, pen.rotation, pen.tiltX, pen.tiltY))
    } else {
        None
    };
    let mut at = POINT { x: info.ptPixelLocation.x, y: info.ptPixelLocation.y };
    let _ = unsafe { ScreenToClient(hwnd, &mut at) };
    let in_contact = info.pointerFlags.0 & POINTER_FLAG_INCONTACT.0 != 0;
    Some(PointerEvent { id, kind, phase, x: at.x as f32 / scale, y: at.y as f32 / scale, in_contact, pen })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pen_axes_follow_the_mask() {
        let full = PenSample::decode(PEN_FLAG_BARREL, 0xf, 512, 90, -30, 45);
        assert_eq!(full.pressure, Some(0.5));
        assert_eq!(full.rotation, Some(90.));
        assert_eq!(full.tilt_x, Some(-30.));
        assert_eq!(full.tilt_y, Some(45.));
        assert!(full.barrel && !full.eraser && !full.inverted);

        let bare = PenSample::decode(PEN_FLAG_ERASER | PEN_FLAG_INVERTED, PEN_MASK_PRESSURE, 2048, 0, 0, 0);
        assert_eq!(bare.pressure, Some(1.));
        assert_eq!((bare.rotation, bare.tilt_x, bare.tilt_y), (None, None, None));
        assert!(bare.eraser && bare.inverted);
    }

    #[test]
    fn pointer_types_map() {
        assert_eq!(PointerKind::from_raw(3), Some(PointerKind::Pen));
        assert_eq!(PointerKind::from_raw(2), Some(PointerKind::Touch));
        assert_eq!(PointerKind::from_raw(1), None);
    }
}
