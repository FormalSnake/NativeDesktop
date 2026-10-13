//! Where the title row's parts are, and what a point in the window hits (ARCHITECTURE.md 5.4).
//!
//! The caption buttons are drawn by the app (Fluent metrics below), yet behave as the system's:
//! the window procedure answers `WM_NCHITTEST` from this layout, so the maximize button reports
//! `HTMAXBUTTON` and Windows 11 opens the Snap Layouts flyout over it. Everything here is in
//! logical pixels from the window's top-left and runs on every OS, so tier 1 covers it.

/// A rectangle in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    /// Half-open on the right and bottom, as a pixel grid is.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CaptionButton {
    Minimize,
    Maximize,
    Close,
}

/// A window edge or corner the pointer resizes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    TopLeft,
    TopRight,
}

/// What a point hits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// App content: the app gets the mouse.
    Client,
    /// A drag region: pointer-down moves the window, double-click maximizes.
    Caption,
    Button(CaptionButton),
    Resize(Edge),
}

/// Fluent caption button width in effective pixels (WinUI `TitleBar`, Windows 11 system frame).
pub const FLUENT_BUTTON_WIDTH: f32 = 46.;
/// Title row height of a standard Fluent window; 48 is the tall variant that holds a search box.
pub const FLUENT_TITLE_HEIGHT: f32 = 32.;
pub const FLUENT_TALL_TITLE_HEIGHT: f32 = 48.;

/// The title row of one window, rebuilt by the backend after each layout pass.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CaptionLayout {
    /// Window client width in logical pixels.
    pub width: f32,
    pub title_height: f32,
    pub buttons: Vec<(CaptionButton, Rect)>,
    /// `drag_region()` rectangles beyond the title row; the title row itself always drags.
    pub drag: Vec<Rect>,
    /// Interactive children inside drag areas (title-row buttons, a search box): they occlude
    /// the caption and get the mouse.
    pub interactive: Vec<Rect>,
    pub resizable: bool,
}

impl CaptionLayout {
    /// The standard Fluent title row: three buttons flush with the top-right corner, each as tall
    /// as the row. The buttons stay flush when maximized, so the corner is a Fitts target.
    pub fn fluent(width: f32, title_height: f32) -> Self {
        let buttons = [CaptionButton::Close, CaptionButton::Maximize, CaptionButton::Minimize]
            .into_iter()
            .enumerate()
            .map(|(i, button)| {
                let x = width - FLUENT_BUTTON_WIDTH * (i + 1) as f32;
                (button, Rect::new(x, 0., FLUENT_BUTTON_WIDTH, title_height))
            })
            .collect();
        Self { width, title_height, buttons, drag: Vec::new(), interactive: Vec::new(), resizable: true }
    }

    pub fn button(&self, which: CaptionButton) -> Option<Rect> {
        self.buttons.iter().find(|(b, _)| *b == which).map(|(_, r)| *r)
    }

    /// The width the caption buttons take from the right of the title row: the trailing inset
    /// `titlebar()` reserves on Windows (ARCHITECTURE.md 3.6).
    pub fn trailing_inset(&self) -> f32 {
        self.buttons.iter().map(|(_, r)| self.width - r.x).fold(0., f32::max)
    }

    /// What `(x, y)` hits. `top_border` is the resize band along the top edge, in logical pixels,
    /// which wins over the caption and the buttons as it does on a system frame; it is ignored
    /// when maximized, where the top edge cannot resize.
    pub fn hit(&self, x: f32, y: f32, maximized: bool, top_border: f32) -> Hit {
        if self.resizable && !maximized && y >= 0. && y < top_border {
            let corner = top_border * 2.;
            return Hit::Resize(if x < corner {
                Edge::TopLeft
            } else if x >= self.width - corner {
                Edge::TopRight
            } else {
                Edge::Top
            });
        }
        if let Some((button, _)) = self.buttons.iter().find(|(_, r)| r.contains(x, y)) {
            return Hit::Button(*button);
        }
        if self.interactive.iter().any(|r| r.contains(x, y)) {
            return Hit::Client;
        }
        let in_title = y >= 0. && y < self.title_height && x >= 0. && x < self.width;
        if in_title || self.drag.iter().any(|r| r.contains(x, y)) {
            return Hit::Caption;
        }
        Hit::Client
    }
}

/// The visual state of the drawn caption buttons, kept by the window procedure from the
/// non-client mouse messages the backend never sees.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CaptionState {
    pub hovered: Option<CaptionButton>,
    pub pressed: Option<CaptionButton>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> CaptionLayout {
        CaptionLayout::fluent(800., FLUENT_TITLE_HEIGHT)
    }

    #[test]
    fn fluent_buttons_sit_flush_top_right() {
        let l = layout();
        assert_eq!(l.button(CaptionButton::Close), Some(Rect::new(754., 0., 46., 32.)));
        assert_eq!(l.button(CaptionButton::Maximize), Some(Rect::new(708., 0., 46., 32.)));
        assert_eq!(l.button(CaptionButton::Minimize), Some(Rect::new(662., 0., 46., 32.)));
        assert_eq!(l.trailing_inset(), 138.);
    }

    #[test]
    fn maximize_button_hits_as_maximize() {
        let l = layout();
        assert_eq!(l.hit(730., 16., false, 4.), Hit::Button(CaptionButton::Maximize));
        // Edges of the button: first column in, last column in, next column is close.
        assert_eq!(l.hit(708., 16., false, 4.), Hit::Button(CaptionButton::Maximize));
        assert_eq!(l.hit(753.9, 16., false, 4.), Hit::Button(CaptionButton::Maximize));
        assert_eq!(l.hit(754., 16., false, 4.), Hit::Button(CaptionButton::Close));
        assert_eq!(l.hit(730., 32., false, 4.), Hit::Client);
    }

    #[test]
    fn top_band_resizes_unless_maximized() {
        let l = layout();
        assert_eq!(l.hit(730., 1., false, 4.), Hit::Resize(Edge::Top));
        assert_eq!(l.hit(799., 1., false, 4.), Hit::Resize(Edge::TopRight));
        assert_eq!(l.hit(2., 1., false, 4.), Hit::Resize(Edge::TopLeft));
        assert_eq!(l.hit(730., 1., true, 4.), Hit::Button(CaptionButton::Maximize));
        let mut fixed = layout();
        fixed.resizable = false;
        assert_eq!(fixed.hit(730., 1., false, 4.), Hit::Button(CaptionButton::Maximize));
    }

    #[test]
    fn title_row_drags_and_interactive_children_occlude_it() {
        let mut l = layout();
        assert_eq!(l.hit(300., 16., false, 4.), Hit::Caption);
        l.interactive.push(Rect::new(280., 4., 120., 24.));
        assert_eq!(l.hit(300., 16., false, 4.), Hit::Client);
        assert_eq!(l.hit(500., 16., false, 4.), Hit::Caption);
        assert_eq!(l.hit(300., 100., false, 4.), Hit::Client);
        l.drag.push(Rect::new(0., 32., 200., 400.));
        assert_eq!(l.hit(100., 100., false, 4.), Hit::Caption);
    }
}
