//! The token set every drawn forms control reads. One struct, one constructor per skin: the
//! Linux (libadwaita) look is [`FormLook::adwaita`], Windows' is [`FormLook::fluent`], and
//! [`FormLook::of`] picks by the theme's preset; the controls draw unchanged.

use nd_components::forms::TextStyle;
use nd_gpui::gpui::{self, FontWeight, Hsla, Pixels, px};
use nd_theme::color::with_alpha;
use nd_theme::{TextStyles, Theme};

#[derive(Clone, Copy, Debug)]
pub struct FormLook {
    /// The look these values are for.
    pub preset: nd_theme::Preset,
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
    /// Text selection behind the selected run, and the selected run's colour when the look
    /// redraws it (WinUI: solid accent under white text).
    pub selection: Hsla,
    pub selection_text: Option<Hsla>,
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
    /// Size, weight and line height per text style: the theme's.
    pub type_scale: TextStyles,
    /// Unfilled track under the pointer, and while pressed.
    pub track_hover: Hsla,
    pub track_active: Hsla,
    /// Thickness of a progress bar's trough.
    pub progress_thickness: Pixels,
    /// The slider and switch knob's drop shadow (libadwaita's `0 2px 4px` of 20% shade), in
    /// place of `knob_edge`.
    pub knob_shadow: Option<Hsla>,
    /// The knob under the pointer.
    pub knob_hover: Hsla,
    /// Checks, switches and sliders darken their trough under the pointer and while pressed,
    /// as libadwaita's do; WinUI's keep theirs.
    pub trough_states: bool,
    /// Height of a menu row, and its fill under the pointer and under the keyboard.
    pub menu_row_height: Pixels,
    pub menu_hover: Hsla,
    pub menu_highlight: Hsla,
    /// Whether a focus ring settles onto its control (libadwaita's 200 ms outline transition)
    /// rather than appearing at once.
    pub ring_settles: bool,
    /// A field's outline at rest: WinUI's control stroke; libadwaita's entries have none.
    pub field_stroke: Option<Hsla>,
    /// An off switch outlined in this colour over the plain fill, its knob in `knob_off`:
    /// WinUI's toggle switch. libadwaita fills the track instead.
    pub switch_outline: Option<Hsla>,
    pub knob_off: Hsla,
    /// WinUI's two-ring focus visual: `focus_ring` is the outer 2 px ring and this the 2 px
    /// ring inside it, drawn around the control without taking layout space. libadwaita
    /// has one ring in the layout instead.
    pub focus_inner: Option<Hsla>,
    /// The bottom edge of a control's elevation border (WinUI's ControlElevationBorderBrush:
    /// ControlStrokeColorSecondary under ControlStrokeColorDefault).
    pub stroke_bottom: Option<Hsla>,
    /// A field's bottom edge at rest (TextControlElevationBorderBrush); focused it is a 2 px
    /// accent underline over `field_focus_fill`.
    pub field_underline: Option<Hsla>,
    pub field_focus_fill: Hsla,
    /// Box of an unchecked checkbox or radio button at rest, hovered and pressed
    /// (ControlAltFillColorSecondary, Tertiary, Quarternary), outlined in `check_stroke`.
    pub check_fill: [Hsla; 3],
    pub check_stroke: Hsla,
    pub check_stroke_width: Pixels,
    /// Side of the check mark in a checked box (WinUI's CheckBoxGlyphSize).
    pub check_glyph: Pixels,
    /// The drop-down chevron of pop-up buttons (WinUI: E70D at 12 px in the secondary text).
    pub chevron_size: Pixels,
    pub chevron_color: Hsla,
    /// Fills of list and menu items hovered and pressed (SubtleFillColorSecondary, Tertiary).
    pub subtle_hover: Hsla,
    pub subtle_active: Hsla,
    /// The current item of a pop-up's menu shows WinUI's 3x16 accent pill at its leading edge
    /// instead of a trailing check mark.
    pub item_pill: bool,
    /// Space between a pop-up's edge and its items across and along, and between items
    /// (WinUI's ComboBox: 5 and 6, 4 between its 32 px items).
    pub popover_pad: (Pixels, Pixels),
    pub item_gap: Pixels,
    /// A pop-up button's menu opens over the button with the current item on it, as WinUI's
    /// ComboBox does, instead of below it.
    pub menu_over: bool,
    /// The pointer over a clickable control: Windows keeps the arrow (only links take the hand).
    pub cursor: gpui::CursorStyle,
    /// A link draws WinUI's HyperlinkButton: button padding, a subtle fill under the pointer
    /// and no underline.
    pub link_button: bool,
    /// A stepper draws WinUI's NumberBox: up and down chevrons inside the field.
    pub spin_inline: bool,
    /// A checkbox's and a radio button's least height (WinUI: 32), and a radio group's spacing
    /// between rows and between columns (RadioButtonsRowSpacing 8, ColumnSpacing 7).
    pub toggle_row: Pixels,
    pub radio_spacing: (Pixels, Pixels),
    /// Flyout outline (SurfaceStrokeColorFlyout; libadwaita's 14% black), and its drop shadow's
    /// colour, y offset and blur (libadwaita's two layers are in [`FormLook::popover_shadows`]).
    pub popover_stroke: Hsla,
    pub popover_shadow: (Hsla, Pixels, Pixels),
    /// Accent fills hovered and pressed (AccentFillColorSecondary and Tertiary).
    pub accent_hover: Hsla,
    pub accent_active: Hsla,
    /// Diameter of the accent dot in a slider thumb (and in a checked radio button) at rest,
    /// hovered and pressed: WinUI's thumb grows its inner dot under the pointer. `None` draws a
    /// plain knob.
    pub thumb_dot: Option<[Pixels; 3]>,
    /// The slider thumb around the dot (ControlSolidFillColorDefault).
    pub thumb_fill: Hsla,
    /// The switch knob grows by this under the pointer, and stretches by `knob_press` along the
    /// track while pressed.
    pub knob_hover_grow: Pixels,
    pub knob_press: Pixels,
    /// A progress bar's rail and bar thickness when the rail is thinner than the bar (WinUI's
    /// 1 px rail under a 3 px bar); `None` draws both `track_thickness` thick.
    pub progress_rail: Option<(Pixels, Pixels)>,
    /// Whether the spinner draws a track ring under its arc.
    pub spinner_track: bool,
}

impl FormLook {
    /// The Linux look on libadwaita's stylesheet: 34 px controls at the 9 px button radius,
    /// fills of the text colour at 10% (15% hovered, 30% pressed), troughs at 15% (20% hovered,
    /// 25% pressed), a 46x26 switch and 20 px checks, a 6 px slider trough and 8 px progress
    /// trough, knobs with a soft drop shadow, 15 px popovers with a 6 px item inset, 0.5 opacity
    /// when insensitive, 200 ms ease-out-quad state transitions. Every colour is the resolved
    /// theme's.
    pub fn adwaita(theme: &Theme) -> Self {
        let ink = theme.text.primary;
        let tint = |share: f32| with_alpha(ink, ink.a * share);
        // libadwaita's slider: 80% white over the view, white when hovered.
        let white = nd_gpui::gpui::white();
        let view = if theme.is_dark() { nd_gpui::gpui::rgb(0x1d1d20).into() } else { white };
        Self {
            preset: theme.preset,
            control_height: px(34.),
            radius: theme.radius.md,
            popover_radius: theme.radius.xl,
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
            accent_text: theme.text.accent,
            selection: theme.selection,
            selection_text: None,
            focus_ring: theme.focus_ring,
            danger: theme.text.danger,
            track: tint(0.15),
            track_hover: tint(0.2),
            track_active: tint(0.25),
            knob: nd_theme::color::mix(view, white, 0.8),
            knob_hover: white,
            knob_edge: nd_gpui::gpui::transparent_black(),
            knob_shadow: Some(nd_gpui::gpui::hsla(240. / 360., 1., 0.012, 0.2)),
            check_size: px(20.),
            check_radius: theme.radius.sm,
            trough_states: true,
            switch_width: px(46.),
            switch_height: px(26.),
            switch_inset: px(3.),
            // `scale > trough`: 4 px.
            track_thickness: px(4.),
            progress_thickness: px(8.),
            slider_knob: px(20.),
            tick_length: px(5.),
            // GtkProgressBar's 150 px trough.
            slider_min_length: px(150.),
            // AdwSpinner at its 16 px.
            spinner_size: px(16.),
            spinner_stroke: px(2.),
            field_min_width: px(120.),
            swatch_size: px(22.),
            day_size: px(30.),
            disabled_opacity: 0.5,
            toggle_seconds: 0.15,
            toggle_ease: [0.23, 1., 0.32, 1.],
            indeterminate_seconds: 1.2,
            indeterminate_segment: 0.3,
            spinner_seconds: 1.2,
            // `modelbutton`: 32 px rows at the button radius, 10% of the text colour hovered or
            // selected.
            menu_row_height: px(32.),
            menu_hover: tint(0.1),
            menu_highlight: tint(0.1),
            ring_settles: true,
            type_scale: theme.text_styles,
            field_stroke: None,
            switch_outline: None,
            knob_off: nd_theme::color::mix(view, white, 0.8),
            focus_inner: None,
            stroke_bottom: None,
            field_underline: None,
            field_focus_fill: tint(0.1),
            check_fill: [gpui::transparent_black(); 3],
            // libadwaita's check button: an inset ring of 15% of the text colour.
            check_stroke: tint(0.15),
            check_stroke_width: px(2.),
            check_glyph: px(14.),
            chevron_size: px(16.),
            chevron_color: theme.text.primary,
            subtle_hover: tint(0.15),
            subtle_active: tint(0.3),
            item_pill: false,
            popover_pad: (px(6.), px(6.)),
            item_gap: px(0.),
            menu_over: false,
            cursor: gpui::CursorStyle::PointingHand,
            link_button: false,
            spin_inline: false,
            toggle_row: px(0.),
            radio_spacing: (px(4.), px(16.)),
            popover_stroke: nd_gpui::gpui::black().opacity(0.14),
            popover_shadow: (gpui::hsla(0., 0., 0., 0.18), px(4.), px(12.)),
            // `suggested-action` and checked toggles: a 10% veil of the ink hovered, a 20% shade pressed.
            accent_hover: nd_theme::color::mix(theme.accent, theme.text.on_accent, 0.1),
            accent_active: nd_theme::color::mix(theme.accent, gpui::black(), 0.2),
            thumb_dot: None,
            thumb_fill: theme.text.on_accent,
            knob_hover_grow: px(0.),
            knob_press: px(0.),
            progress_rail: None,
            spinner_track: true,
        }
    }

    /// The look of `theme`'s preset.
    pub fn of(theme: &Theme) -> Self {
        match theme.preset {
            nd_theme::Preset::Fluent => Self::fluent(theme),
            nd_theme::Preset::Adwaita => Self::adwaita(theme),
            nd_theme::Preset::MacOs | nd_theme::Preset::Plain => Self::plain(theme),
        }
    }

    /// The drawn look off Linux and Windows (macOS's GPUI backend, the web): libadwaita's
    /// metrics without its stylesheet's colours and states, so libadwaita-only rules stay on
    /// Linux.
    pub fn plain(theme: &Theme) -> Self {
        let adwaita = Self::adwaita(theme);
        Self {
            radius: px(6.),
            popover_radius: px(12.),
            accent_text: theme.accent,
            track: theme.border.strong,
            track_hover: theme.border.strong,
            track_active: theme.border.strong,
            knob: theme.text.on_accent,
            knob_hover: theme.text.on_accent,
            knob_off: theme.text.on_accent,
            knob_edge: theme.border.strong,
            knob_shadow: None,
            check_size: px(18.),
            check_radius: px(5.),
            check_stroke: theme.border.strong,
            trough_states: false,
            progress_thickness: adwaita.track_thickness,
            slider_min_length: px(140.),
            spinner_size: px(18.),
            spinner_seconds: 0.9,
            menu_row_height: adwaita.control_height,
            menu_hover: adwaita.fill_hover,
            menu_highlight: adwaita.fill_active,
            ring_settles: false,
            popover_stroke: theme.border.subtle,
            accent_hover: theme.accent,
            accent_active: theme.accent,
            ..adwaita
        }
    }

    /// WinUI 3's controls, from its theme resources (microsoft-ui-xaml `CommonStyles`): 32 px
    /// tall at the 4 px control radius, 8 px flyouts, fills a few percent of white over the
    /// layer (ControlFillColorDefault, Secondary, Tertiary) inside an elevation border, a 40x20
    /// toggle switch with a 12 px knob, 20 px check boxes outlined in the strong stroke, an 18 px
    /// slider thumb (20 px with its border) around a 12 px accent dot, a 1 px progress rail under a 3 px bar.
    pub fn fluent(theme: &Theme) -> Self {
        // From tokens, so the appearance audit can trace them: the card's white over a light
        // layer, the text's white thinly over a dark one.
        let dark = theme.is_dark();
        let (ink, card, base) = (theme.text.primary, theme.surface.raised, theme.surface.base);
        let black = gpui::black();
        let (fill, fill_hover, fill_active) = if dark {
            (with_alpha(ink, 0.06), with_alpha(ink, 0.08), with_alpha(ink, 0.03))
        } else {
            (with_alpha(card, 0.7), with_alpha(base, 0.5), with_alpha(base, 0.3))
        };
        // ControlStrongStrokeColorDefault: #72000000 light, #8BFFFFFF dark.
        let strong = if dark { with_alpha(ink, 0.545) } else { with_alpha(black, 0.447) };
        // ControlAltFillColorSecondary, Tertiary, Quarternary.
        let check_fill = if dark {
            [with_alpha(black, 0.1), with_alpha(ink, 0.043), with_alpha(ink, 0.07)]
        } else {
            [with_alpha(black, 0.024), with_alpha(black, 0.059), with_alpha(black, 0.094)]
        };
        let accent = theme.accent;
        Self {
            control_height: px(32.),
            radius: theme.radius.md,
            popover_radius: theme.radius.lg,
            popover_inset: px(4.),
            padding_x: px(11.),
            field_padding_x: px(10.),
            padding_y: px(5.),
            fill,
            fill_hover,
            fill_active,
            // ControlStrokeColorDefault.
            border: if dark { with_alpha(ink, 0.07) } else { with_alpha(black, 0.059) },
            check_size: px(20.),
            check_radius: theme.radius.md,
            switch_width: px(40.),
            switch_height: px(20.),
            switch_inset: px(4.),
            // The 18 px thumb inside its 1 px elevation border.
            slider_knob: px(20.),
            tick_length: px(4.),
            spinner_size: px(32.),
            spinner_stroke: px(4.),
            // TextFillColorDisabled is 40% of the primary text.
            disabled_opacity: 0.4,
            // ControlFastAnimationDuration on Fluent's "fast out, slow in" curve.
            toggle_seconds: 0.167,
            toggle_ease: [0., 0., 0., 1.],
            spinner_seconds: 2.,
            field_stroke: Some(if dark { with_alpha(ink, 0.07) } else { with_alpha(black, 0.059) }),
            switch_outline: Some(strong),
            knob_off: theme.text.secondary,
            knob_edge: gpui::transparent_black(),
            track: strong,
            // FocusStrokeColorOuter and Inner.
            focus_ring: if dark { gpui::white() } else { with_alpha(black, 0.894) },
            focus_inner: Some(if dark { with_alpha(black, 0.7) } else { with_alpha(gpui::white(), 0.7) }),
            // ControlStrokeColorSecondary.
            stroke_bottom: Some(if dark { with_alpha(ink, 0.094) } else { with_alpha(black, 0.16) }),
            field_underline: Some(strong),
            // ControlFillColorInputActive.
            field_focus_fill: if dark { gpui::rgba(0x1e1e1eb3).into() } else { gpui::white() },
            check_fill,
            check_stroke: strong,
            check_stroke_width: px(1.),
            check_glyph: px(12.),
            chevron_size: px(12.),
            chevron_color: theme.text.secondary,
            subtle_hover: if dark { with_alpha(ink, 0.059) } else { with_alpha(black, 0.035) },
            subtle_active: if dark { with_alpha(ink, 0.039) } else { with_alpha(black, 0.024) },
            item_pill: true,
            popover_pad: (px(5.), px(6.)),
            item_gap: px(4.),
            menu_over: true,
            cursor: gpui::CursorStyle::Arrow,
            link_button: true,
            spin_inline: true,
            // AccentFillColorSelectedTextBackground under white; in dark the accent container,
            // since the dark accent is too light to carry white.
            selection: if dark { theme.accent_container.bg } else { theme.accent },
            selection_text: Some(gpui::white()),
            toggle_row: px(32.),
            radio_spacing: (px(8.), px(7.)),
            popover_stroke: if dark { with_alpha(black, 0.2) } else { with_alpha(black, 0.059) },
            // The flyout's ThemeShadow, as one blur.
            popover_shadow: (gpui::hsla(0., 0., 0., if dark { 0.26 } else { 0.14 }), px(8.), px(16.)),
            accent_hover: with_alpha(accent, accent.a * 0.9),
            accent_active: with_alpha(accent, accent.a * 0.8),
            thumb_dot: Some([px(12.), px(14.), px(10.)]),
            thumb_fill: if dark { gpui::rgb(0x454545).into() } else { gpui::white() },
            knob_hover_grow: px(2.),
            knob_press: px(3.),
            progress_rail: Some((px(1.), px(3.))),
            track_thickness: px(4.),
            spinner_track: false,
            knob: theme.text.on_accent,
            knob_hover: theme.text.on_accent,
            knob_shadow: None,
            track_hover: strong,
            track_active: strong,
            trough_states: false,
            slider_min_length: px(140.),
            menu_hover: fill_hover,
            menu_highlight: fill_active,
            ring_settles: false,
            ..Self::adwaita(theme)
        }
    }

    /// A popover's shadow: libadwaita's near 9% and far 5% layers, elsewhere `popover_shadow`.
    pub fn popover_shadows(&self) -> Vec<nd_gpui::gpui::BoxShadow> {
        use nd_gpui::gpui::{BoxShadow, hsla, point};
        let layer = |alpha: f32, y: f32, blur: f32, spread: f32| BoxShadow {
            color: hsla(0., 0., 0., alpha),
            offset: point(px(0.), px(y)),
            blur_radius: px(blur),
            spread_radius: px(spread),
            inset: false,
        };
        if self.preset == nd_theme::Preset::Adwaita {
            vec![layer(0.09, 1., 5., 1.), layer(0.05, 2., 14., 3.)]
        } else {
            let (color, y, blur) = self.popover_shadow;
            vec![BoxShadow { color, offset: point(px(0.), y), blur_radius: blur, spread_radius: px(0.), inset: false }]
        }
    }

    pub fn type_of(&self, style: TextStyle) -> (Pixels, FontWeight, Pixels) {
        let t = self.type_scale[style];
        (t.size, t.weight, t.line_height)
    }

    /// Progress of an eased transition `t` (0 to 1) along `toggle_ease`.
    pub fn ease(&self, t: f32) -> f32 {
        cubic_bezier(self.toggle_ease, t.clamp(0., 1.))
    }
}

/// What a libadwaita push button is (`_buttons.scss`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdwButton {
    /// The text colour at 10%, 15% hovered, 30% pressed.
    Regular,
    /// `.flat`: nothing at rest, 7% hovered, 16% pressed.
    Flat,
    /// `.suggested-action`: the accent, a 10% veil of its ink hovered, 20% shade pressed.
    Suggested,
    /// `.destructive-action`: the destructive colour as text over 15% of itself, 20% hovered,
    /// 35% pressed.
    Destructive,
}

/// A button's fills at rest, hovered and pressed, and its ink.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonFills {
    pub rest: Hsla,
    pub hover: Hsla,
    pub active: Hsla,
    pub ink: Hsla,
}

impl AdwButton {
    pub fn fills(self, theme: &Theme) -> ButtonFills {
        let tint = |c: Hsla, share: f32| with_alpha(c, c.a * share);
        match self {
            AdwButton::Regular | AdwButton::Flat => {
                let ink = theme.text.primary;
                let (rest, hover, active) = if self == AdwButton::Flat { (0., 0.07, 0.16) } else { (0.1, 0.15, 0.3) };
                ButtonFills { rest: tint(ink, rest), hover: tint(ink, hover), active: tint(ink, active), ink }
            }
            AdwButton::Suggested => {
                // `accent_fill`: libadwaita's accent background, darkened only as far as white
                // labels need to read at 4.5:1 (its own #3584e4 gives 3.8:1).
                let (fill, ink) = (theme.accent_fill, theme.text.on_accent);
                let shade = nd_gpui::gpui::black();
                ButtonFills {
                    rest: fill,
                    hover: nd_theme::color::mix(fill, with_alpha(ink, fill.a), 0.1),
                    active: nd_theme::color::mix(fill, with_alpha(shade, fill.a), 0.2),
                    ink,
                }
            }
            AdwButton::Destructive => {
                let red = theme.text.danger;
                ButtonFills { rest: tint(red, 0.15), hover: tint(red, 0.2), active: tint(red, 0.35), ink: red }
            }
        }
    }
}

/// libadwaita's `$ease_out_quad`, the curve of its state and focus transitions.
pub const EASE_OUT_QUAD: [f32; 4] = [0.25, 0.46, 0.45, 0.94];

/// y at x on the curve from (0, 0) to (1, 1) with control points `c`, solved for x by bisection.
pub(crate) fn cubic_bezier(c: [f32; 4], x: f32) -> f32 {
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
