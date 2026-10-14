//! The token set every drawn forms control reads. One struct, one constructor per skin: the
//! Linux (libadwaita) look is [`FormLook::adwaita`]; a Fluent skin supplies its own constructor
//! and the controls draw unchanged.

use nd_components::forms::TextStyle;
use nd_gpui::gpui::{FontWeight, Hsla, Pixels, px};
use nd_theme::Theme;
use nd_theme::color::with_alpha;

#[derive(Clone, Copy, Debug)]
pub struct FormLook {
    /// Height of single-line controls: fields, pop-up buttons, segmented controls, steppers.
    pub control_height: Pixels,
    /// Corner radius of fields, buttons, segments and popover items.
    pub radius: Pixels,
    /// Corner radius of popover surfaces: `radius` plus `popover_inset`, so corners nest.
    pub popover_radius: Pixels,
    /// Space between a popover's edge and its items.
    pub popover_inset: Pixels,
    /// Horizontal padding inside buttons.
    pub padding_x: Pixels,
    /// Horizontal padding inside text fields, inside the 2 px focus ring border.
    pub field_padding_x: Pixels,
    /// Vertical padding inside multi-line fields.
    pub padding_y: Pixels,
    /// Space between a control's parts (indicator and title, icon and text).
    pub gap: Pixels,
    /// Body text size of every control.
    pub text_size: Pixels,
    /// Line height of body text.
    pub line_height: Pixels,
    /// Size of field edge icons and stepper arrows.
    pub icon_size: Pixels,
    /// Stroke of drawn glyphs (check marks, arrows, the magnifier).
    pub glyph_stroke: Pixels,
    /// Window text.
    pub text: Hsla,
    /// Supporting text: placeholders, secondary labels, tick marks.
    pub secondary_text: Hsla,
    /// Resting fill of fields and buttons: a tint of the text colour, so a control reads on the
    /// window and on a grouped list alike.
    pub fill: Hsla,
    /// Fill under the pointer.
    pub fill_hover: Hsla,
    /// Fill while pressed, and of the selected segment.
    pub fill_active: Hsla,
    /// Hairline around popovers and the selected segment.
    pub border: Hsla,
    /// Popover and calendar surface.
    pub popover: Hsla,
    /// The selected toggle of a segmented control, raised off its group's fill.
    pub raised: Hsla,
    /// Selected and on-state fill: checked boxes, switches on, slider fill, progress.
    pub accent: Hsla,
    /// Content drawn on `accent` (check marks, the selected day).
    pub on_accent: Hsla,
    /// Accent as text on the window background (links).
    pub accent_text: Hsla,
    /// Text selection behind the selected run.
    pub selection: Hsla,
    /// Keyboard focus ring, drawn 2 px wide.
    pub focus_ring: Hsla,
    /// Errors and destructive consequences (danger labels).
    pub danger: Hsla,
    /// Unfilled track of switches, sliders and progress bars, and the box of an unchecked check.
    pub track: Hsla,
    /// Slider and switch knobs.
    pub knob: Hsla,
    /// Outline giving the knob an edge against the track.
    pub knob_edge: Hsla,
    /// Side of checkboxes and radio buttons.
    pub check_size: Pixels,
    /// Corner radius of the checkbox.
    pub check_radius: Pixels,
    /// Switch track width and height.
    pub switch_width: Pixels,
    pub switch_height: Pixels,
    /// Space between the switch track and its knob.
    pub switch_inset: Pixels,
    /// Thickness of slider tracks and progress bars.
    pub track_thickness: Pixels,
    /// Diameter of the slider knob.
    pub slider_knob: Pixels,
    /// Length of slider tick marks.
    pub tick_length: Pixels,
    /// Shortest slider, along its axis.
    pub slider_min_length: Pixels,
    /// Diameter of the spinner and of circular progress.
    pub spinner_size: Pixels,
    /// Stroke of the spinner and of circular progress.
    pub spinner_stroke: Pixels,
    /// Width of the field part of a date picker, colour well hex field and stepper.
    pub field_min_width: Pixels,
    /// Side of a colour well swatch and a palette swatch.
    pub swatch_size: Pixels,
    /// Side of a day cell in the graphical date picker.
    pub day_size: Pixels,
    /// Opacity of a disabled control.
    pub disabled_opacity: f32,
    /// Duration of the switch knob's slide, in seconds.
    pub toggle_seconds: f64,
    /// Easing of the switch knob's slide, as cubic-bezier control points.
    pub toggle_ease: [f32; 4],
    /// Seconds for one pass of the indeterminate bar.
    pub indeterminate_seconds: f64,
    /// Length of the indeterminate bar's moving segment, as a share of the track.
    pub indeterminate_segment: f32,
    /// Seconds for one turn of the spinner.
    pub spinner_seconds: f64,
    /// Size, weight and line height per text style, in [`TextStyle`] declaration order.
    pub type_scale: [(f32, FontWeight, f32); 11],
}

impl FormLook {
    /// The Linux look on libadwaita metrics: 34 px controls, 6 px radius, fills of the text colour
    /// at 10% (15% hovered, 30% pressed), 46x26 switch with a 20 px knob, 4 px slider track with a
    /// 20 px knob, 12 px popovers with 6 px item inset, 0.5 opacity when insensitive. Every colour is the resolved theme's.
    pub fn adwaita(theme: &Theme) -> Self {
        let ink = theme.text.primary;
        let tint = |share: f32| with_alpha(ink, ink.a * share);
        Self {
            control_height: px(34.),
            radius: px(6.),
            popover_radius: px(12.),
            popover_inset: px(6.),
            padding_x: px(10.),
            // libadwaita's 9 px entry padding less the ring border.
            field_padding_x: px(7.),
            padding_y: px(7.),
            gap: px(8.),
            text_size: px(14.),
            line_height: px(20.),
            icon_size: px(16.),
            glyph_stroke: px(1.75),
            text: theme.text.primary,
            secondary_text: theme.text.secondary,
            fill: tint(0.1),
            fill_hover: tint(0.15),
            fill_active: tint(0.3),
            border: theme.border.subtle,
            popover: theme.surface.overlay,
            raised: theme.surface.raised,
            accent: theme.accent,
            on_accent: theme.text.on_accent,
            accent_text: theme.accent,
            selection: theme.selection,
            focus_ring: theme.focus_ring,
            danger: theme.text.danger,
            track: theme.border.strong,
            knob: theme.text.on_accent,
            knob_edge: theme.border.strong,
            check_size: px(18.),
            check_radius: px(5.),
            switch_width: px(46.),
            switch_height: px(26.),
            switch_inset: px(3.),
            track_thickness: px(4.),
            slider_knob: px(20.),
            tick_length: px(5.),
            slider_min_length: px(140.),
            spinner_size: px(18.),
            spinner_stroke: px(2.),
            field_min_width: px(120.),
            swatch_size: px(22.),
            day_size: px(30.),
            disabled_opacity: 0.5,
            toggle_seconds: 0.15,
            toggle_ease: [0.23, 1., 0.32, 1.],
            indeterminate_seconds: 1.2,
            indeterminate_segment: 0.3,
            spinner_seconds: 0.9,
            type_scale: [
                (26., FontWeight::EXTRA_BOLD, 32.),
                (20., FontWeight::EXTRA_BOLD, 26.),
                (20., FontWeight::BOLD, 26.),
                (17., FontWeight::BOLD, 22.),
                (14., FontWeight::BOLD, 20.),
                (13., FontWeight::NORMAL, 18.),
                (14., FontWeight::NORMAL, 20.),
                (13., FontWeight::NORMAL, 18.),
                (12., FontWeight::NORMAL, 16.),
                (12., FontWeight::NORMAL, 16.),
                (11., FontWeight::NORMAL, 14.),
            ],
        }
    }

    pub fn type_of(&self, style: TextStyle) -> (Pixels, FontWeight, Pixels) {
        let index = match style {
            TextStyle::LargeTitle => 0,
            TextStyle::Title1 => 1,
            TextStyle::Title2 => 2,
            TextStyle::Title3 => 3,
            TextStyle::Headline => 4,
            TextStyle::Subheadline => 5,
            TextStyle::Body => 6,
            TextStyle::Callout => 7,
            TextStyle::Footnote => 8,
            TextStyle::Caption1 => 9,
            TextStyle::Caption2 => 10,
        };
        let (size, weight, line_height) = self.type_scale[index];
        (px(size), weight, px(line_height))
    }

    /// Progress of an eased transition `t` (0 to 1) along `toggle_ease`.
    pub fn ease(&self, t: f32) -> f32 {
        cubic_bezier(self.toggle_ease, t.clamp(0., 1.))
    }
}

/// y at x on the curve from (0, 0) to (1, 1) with control points `c`, solved for x by bisection.
fn cubic_bezier(c: [f32; 4], x: f32) -> f32 {
    let at = |a: f32, b: f32, t: f32| {
        let u = 1. - t;
        3. * u * u * t * a + 3. * u * t * t * b + t * t * t
    };
    let (mut lo, mut hi) = (0f32, 1f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.;
        if at(c[0], c[2], mid) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    at(c[1], c[3], (lo + hi) / 2.)
}
