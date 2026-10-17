/// One scroll axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Axes {
    pub horizontal: bool,
    pub vertical: bool,
}

impl Axes {
    pub const NONE: Axes = Axes { horizontal: false, vertical: false };
    pub const BOTH: Axes = Axes { horizontal: true, vertical: true };

    pub fn has(self, axis: Axis) -> bool {
        match axis {
            Axis::Horizontal => self.horizontal,
            Axis::Vertical => self.vertical,
        }
    }

    pub fn any(self) -> bool {
        self.horizontal || self.vertical
    }
}

/// How far each edge of a row swipes open, in points: the width of its actions.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RowSwipeConfig {
    pub leading: f32,
    pub trailing: f32,
    /// Swiping this edge past half the row's width commits its first action.
    pub full_swipe: Option<crate::RowEdge>,
}

/// The gestures one node listens for. Built by the element modifiers in `nd-ui`; compared by
/// value, so an unchanged set produces no backend update.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GestureSet {
    pub tap: bool,
    pub double_tap: bool,
    pub long_press: bool,
    /// `Some(None)` drags freely, `Some(Some(axis))` along one axis.
    pub drag: Option<Option<Axis>>,
    pub pinch: bool,
    pub rotate: bool,
    pub swipe_page: bool,
    pub force_click: bool,
    pub smart_magnify: bool,
    pub row_swipe: Option<RowSwipeConfig>,
    /// Axes the node scrolls on (its style's `overflow: scroll`).
    pub scroll: Axes,
}

impl GestureSet {
    pub fn is_empty(&self) -> bool {
        *self == GestureSet::default()
    }

    /// Takes part in the pointer arena.
    pub fn has_pointer(&self) -> bool {
        self.tap || self.double_tap || self.long_press || self.drag.is_some()
    }

    /// Takes part in two-finger scroll arbitration.
    pub fn has_scroll(&self) -> bool {
        self.scroll.any() || self.swipe_page || self.row_swipe.is_some()
    }
}

/// Thresholds. Backends fill the OS values in (`NSEvent.doubleClickInterval`, the GTK/Windows
/// double-click time); the defaults match macOS.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GestureConfig {
    /// Movement before a press stops being a tap and a drag can claim it, in points.
    pub slop: f32,
    /// Movement before a two-finger scroll locks to an axis, in points.
    pub lock_distance: f32,
    pub long_press: f64,
    pub double_tap: f64,
    /// Seconds a notched wheel's step eases over; 0 jumps. WinUI's ScrollViewer eases a wheel
    /// notch, so Windows builds glide (120 ms); GTK and AppKit move the content by the step.
    pub wheel_glide: f64,
}

impl Default for GestureConfig {
    fn default() -> Self {
        Self {
            slop: 4.,
            lock_distance: 8.,
            long_press: 0.5,
            double_tap: 0.5,
            wheel_glide: if cfg!(target_os = "windows") { 0.12 } else { 0. },
        }
    }
}
