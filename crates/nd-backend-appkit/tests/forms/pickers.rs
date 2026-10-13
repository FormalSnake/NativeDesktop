//! Date picker, colour well, progress, label and link on their AppKit classes.

use nd_backend_appkit::objc2::rc::Retained;
use nd_backend_appkit::objc2::runtime::AnyObject;
use nd_backend_appkit::objc2_app_kit::{
    NSAccessibility, NSButton, NSColor, NSColorSpace, NSColorWell, NSColorWellStyle, NSControl, NSDatePicker,
    NSDatePickerElementFlags, NSDatePickerStyle, NSFont, NSFontAttributeName, NSFontTextStyle, NSFontTextStyleBody,
    NSFontTextStyleCallout, NSFontTextStyleCaption1, NSFontTextStyleCaption2, NSFontTextStyleFootnote,
    NSFontTextStyleHeadline, NSFontTextStyleLargeTitle, NSFontTextStyleSubheadline, NSFontTextStyleTitle1,
    NSFontTextStyleTitle2, NSFontTextStyleTitle3, NSForegroundColorAttributeName, NSLineBreakMode, NSProgressIndicator,
    NSProgressIndicatorStyle, NSTextField, NSView,
};
use nd_backend_appkit::objc2_foundation::{
    NSCalendar, NSCalendarIdentifierGregorian, NSDate, NSDateComponents, NSDictionary,
};
use nd_components::*;
use nd_ui::prelude::*;
use nd_ui::semantics::SemanticAction;
use nd_ui::{Role, WindowId};

use super::{Checks, Rig};

const FIRST: CivilDateTime = CivilDateTime::date(2026, 3, 4);
const MIN: CivilDateTime = CivilDateTime::date(2026, 1, 1);
const MAX: CivilDateTime = CivilDateTime::date(2026, 12, 31);

struct View {
    date: CivilDateTime,
    date_events: Vec<CivilDateTime>,
    rewrite_date: bool,
    disabled: bool,
    color: Hsla,
    alpha: bool,
    color_events: Vec<Hsla>,
    rewrite_color: bool,
    progress: Option<f32>,
    link_clicks: Vec<Option<SharedString>>,
    wrap: bool,
}

impl View {
    fn new() -> Self {
        Self {
            date: FIRST,
            date_events: Vec::new(),
            rewrite_date: false,
            disabled: false,
            color: Hsla::from(rgb(0x3366ff)),
            alpha: true,
            color_events: Vec::new(),
            rewrite_color: false,
            progress: Some(0.4),
            link_clicks: Vec::new(),
            wrap: false,
        }
    }
}

const STYLES: [TextStyle; 11] = [
    TextStyle::LargeTitle,
    TextStyle::Title1,
    TextStyle::Title2,
    TextStyle::Title3,
    TextStyle::Headline,
    TextStyle::Subheadline,
    TextStyle::Body,
    TextStyle::Callout,
    TextStyle::Footnote,
    TextStyle::Caption1,
    TextStyle::Caption2,
];

impl Render for View {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut column = v_flex().size_full().p_4().gap_2();
        for (i, style) in STYLES.into_iter().enumerate() {
            column = column.child(label(format!("Style {i}")).id(format!("style-{i}")).text_style(style));
        }
        column
            .child(label("Secondary").id("secondary").secondary())
            .child(label("Danger").id("danger").tone(LabelTone::Danger))
            .child(label("Selectable").id("selectable").selectable())
            .child({
                let l = label("A long line of text that has to wrap inside its width.").id("wrapping").w(px(120.));
                if self.wrap { l.wraps() } else { l }
            })
            .child(
                date_picker("date", self.date)
                    .mode(DatePickerMode::Date)
                    .min(MIN)
                    .max(MAX)
                    .label("Due")
                    .tooltip("Pick a day")
                    .disabled(self.disabled)
                    .on_change(cx.listener(|this, e: &DateChanged, _, cx| {
                        this.date_events.push(e.value);
                        this.date = if this.rewrite_date {
                            CivilDateTime::date(e.value.year, e.value.month, 1)
                        } else {
                            e.value
                        };
                        cx.notify();
                    })),
            )
            .child(date_picker("time", FIRST.at(9, 30, 15)).mode(DatePickerMode::Time))
            .child(
                date_picker("both", FIRST.at(9, 30, 15))
                    .mode(DatePickerMode::DateTime)
                    .presentation(DatePickerStyle::Stepper),
            )
            .child(date_picker("graphical", FIRST).presentation(DatePickerStyle::Graphical))
            .child(color_well("color", self.color).alpha(self.alpha).label("Tint").tooltip("Pick a tint").on_change(
                cx.listener(|this, e: &ColorChanged, _, cx| {
                    this.color_events.push(e.color);
                    this.color = if this.rewrite_color { Hsla::from(rgb(0x00ff00)) } else { e.color };
                    cx.notify();
                }),
            ))
            .child(progress(self.progress).id("bar").label("Download").tooltip("Downloading"))
            .child(progress(None).id("bar-indeterminate"))
            .child(spinner().id("spinner"))
            .child(progress(Some(0.25)).id("circular").circular())
            .child(
                link("link", "Read more")
                    .url("https://example.com/docs")
                    .keep_internal()
                    .tooltip("Opens the docs")
                    .disabled(self.disabled)
                    .on_click(cx.listener(|this, e: &LinkActivated, _, _| this.link_clicks.push(e.url.clone()))),
            )
    }
}

fn gregorian() -> Retained<NSCalendar> {
    let calendar = NSCalendar::calendarWithIdentifier(unsafe { NSCalendarIdentifierGregorian }).unwrap();
    calendar.setTimeZone(&NSCalendar::currentCalendar().timeZone());
    calendar
}

fn ns_date(v: CivilDateTime) -> Retained<NSDate> {
    let parts = NSDateComponents::new();
    parts.setYear(v.year as isize);
    parts.setMonth(v.month as isize);
    parts.setDay(v.day as isize);
    parts.setHour(v.hour as isize);
    parts.setMinute(v.minute as isize);
    parts.setSecond(v.second as isize);
    gregorian().dateFromComponents(&parts).unwrap()
}

fn instant(date: &Option<Retained<NSDate>>) -> Option<f64> {
    date.as_ref().map(|d| d.timeIntervalSinceReferenceDate())
}

fn preferred(style: &NSFontTextStyle) -> Retained<NSFont> {
    unsafe { NSFont::preferredFontForTextStyle_options(style, &NSDictionary::new()) }
}

fn expected_font(style: TextStyle) -> Retained<NSFont> {
    unsafe {
        preferred(match style {
            TextStyle::LargeTitle => NSFontTextStyleLargeTitle,
            TextStyle::Title1 => NSFontTextStyleTitle1,
            TextStyle::Title2 => NSFontTextStyleTitle2,
            TextStyle::Title3 => NSFontTextStyleTitle3,
            TextStyle::Headline => NSFontTextStyleHeadline,
            TextStyle::Subheadline => NSFontTextStyleSubheadline,
            TextStyle::Body => NSFontTextStyleBody,
            TextStyle::Callout => NSFontTextStyleCallout,
            TextStyle::Footnote => NSFontTextStyleFootnote,
            TextStyle::Caption1 => NSFontTextStyleCaption1,
            TextStyle::Caption2 => NSFontTextStyleCaption2,
        })
    }
}

fn as_class<T: nd_backend_appkit::objc2::DowncastTarget>(view: Retained<NSView>) -> Option<Retained<T>> {
    view.downcast::<T>().ok()
}

/// What a click or edit in the control does: its own action, sent to its own target.
fn user_change(control: &NSControl) {
    let target = control.target();
    unsafe { control.sendAction_to(control.action(), target.as_deref()) };
}

fn rgba(color: &NSColor) -> Option<[f64; 4]> {
    let c = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    Some([c.redComponent(), c.greenComponent(), c.blueComponent(), c.alphaComponent()])
}

fn near(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
}

fn hsla_rgba(color: Hsla) -> [f64; 4] {
    let c = Rgba::from(color);
    [f64::from(c.r), f64::from(c.g), f64::from(c.b), f64::from(c.a)]
}

fn hsla_close(a: Hsla, b: Hsla) -> bool {
    near(hsla_rgba(a), hsla_rgba(b))
}

fn same_object<T: nd_backend_appkit::objc2::Message>(a: &Retained<T>, b: &Retained<T>) -> bool {
    Retained::as_ptr(a) == Retained::as_ptr(b)
}

pub fn run(rig: &Rig, c: &mut Checks) {
    let opened = rig.open("Pickers", |_| View::new());
    let w = opened.id();
    rig.pump();
    date(rig, c, &opened, w);
    colour(rig, c, &opened, w);
    progress_indicators(rig, c, w);
    labels(rig, c, &opened, w);
    links(rig, c, &opened, w);
    rig.close(opened);
}

type Opened = super::rig::Opened<View>;

fn date(rig: &Rig, c: &mut Checks, opened: &Opened, w: WindowId) {
    let Some(picker) = as_class::<NSDatePicker>(rig.view(w, "date")) else {
        return c.check(false, "date picker mounts as NSDatePicker");
    };
    c.check(true, "date picker mounts as NSDatePicker");
    c.check(picker.datePickerElements() == NSDatePickerElementFlags::YearMonthDay, "date mode shows year, month, day");
    c.check(picker.datePickerStyle() == NSDatePickerStyle::TextField, "field presentation is the text field style");
    c.check(
        instant(&picker.minDate()) == Some(ns_date(MIN).timeIntervalSinceReferenceDate())
            && instant(&picker.maxDate()) == Some(ns_date(MAX).timeIntervalSinceReferenceDate()),
        "min and max reach minDate and maxDate",
    );
    c.check(
        instant(&Some(picker.dateValue())) == Some(ns_date(FIRST).timeIntervalSinceReferenceDate()),
        "value is the civil wall clock in the current zone",
    );
    c.check(picker.toolTip().is_some_and(|t| t.to_string() == "Pick a day"), "date tooltip reaches the view");
    c.check(
        picker.accessibilityLabel().is_some_and(|t| t.to_string() == "Due"),
        "date accessibility label reaches the view",
    );
    c.check(
        as_class::<NSDatePicker>(rig.view(w, "time"))
            .is_some_and(|p| p.datePickerElements() == NSDatePickerElementFlags::HourMinuteSecond),
        "time mode shows hour, minute, second",
    );
    c.check(
        as_class::<NSDatePicker>(rig.view(w, "both")).is_some_and(|p| {
            p.datePickerElements()
                == (NSDatePickerElementFlags::YearMonthDay | NSDatePickerElementFlags::HourMinuteSecond)
                && p.datePickerStyle() == NSDatePickerStyle::TextFieldAndStepper
        }),
        "date and time mode with stepper presentation",
    );
    c.check(
        as_class::<NSDatePicker>(rig.view(w, "graphical"))
            .is_some_and(|p| p.datePickerStyle() == NSDatePickerStyle::ClockAndCalendar),
        "graphical presentation is clock and calendar",
    );
    let fitted = rig.node(w, "date").and_then(|n| n.bounds).is_some_and(|b| b.size.width > 0. && b.size.height > 0.);
    c.check(fitted, "date picker is laid out at the size AppKit measures");

    let node = rig.node(w, "date");
    c.check(
        node.as_ref()
            .is_some_and(|n| n.role == Role::DatePicker && n.value.as_deref() == Some("2026-03-04") && n.enabled),
        "date node has role, value and enabled",
    );

    // A user edit arrives as the event with the value the picker holds.
    let picked = CivilDateTime::date(2026, 5, 6);
    picker.setDateValue(&ns_date(picked));
    user_change(&picker);
    rig.pump();
    c.check(
        rig.read(opened, |v| v.date_events.clone()) == vec![picked],
        "a user date change arrives as DateChanged once",
    );

    // Echo: the app fed the value back; nothing is set on the picker.
    let before = picker.dateValue();
    rig.pump();
    rig.update(opened, |_, cx| cx.notify());
    rig.pump();
    c.check(same_object(&before, &picker.dateValue()), "feeding the reported date back sets nothing");
    c.check(rig.read(opened, |v| v.date_events.len()) == 1, "the echo emits no further event");

    // Normalisation: past max, the picker holds max, one event, no second set.
    rig.update(opened, |v, cx| {
        v.date = CivilDateTime::date(2030, 1, 1);
        cx.notify();
    });
    rig.pump();
    rig.pump();
    let events = rig.read(opened, |v| v.date_events.clone());
    c.check(events == vec![picked, MAX], "a date past max emits exactly one event with the clamped value");
    c.check(
        instant(&Some(picker.dateValue())) == Some(ns_date(MAX).timeIntervalSinceReferenceDate()),
        "the picker holds max",
    );
    let held = picker.dateValue();
    rig.pump();
    c.check(
        same_object(&held, &picker.dateValue()) && rig.read(opened, |v| v.date_events.len()) == 2,
        "the clamped value fed back sets nothing",
    );

    // Reentrancy: the handler writes a different value into the controlled prop itself.
    rig.update(opened, |v, _| v.rewrite_date = true);
    picker.setDateValue(&ns_date(CivilDateTime::date(2026, 8, 20)));
    user_change(&picker);
    rig.pump();
    rig.pump();
    c.check(rig.read(opened, |v| v.date_events.len()) == 3, "a handler that sets the value emits once, no loop");
    c.check(
        instant(&Some(picker.dateValue()))
            == Some(ns_date(CivilDateTime::date(2026, 8, 1)).timeIntervalSinceReferenceDate()),
        "the value the handler wrote reaches the picker",
    );
    rig.update(opened, |v, _| v.rewrite_date = false);

    // Automation acts through the native control.
    let result = rig.perform(w, "date", SemanticAction::SetValue("2026-09-10".into()));
    c.check(result.is_ok(), "SetValue on the date picker succeeds");
    c.check(
        rig.read(opened, |v| v.date_events.last().copied()) == Some(CivilDateTime::date(2026, 9, 10)),
        "SetValue reaches the app as DateChanged",
    );
    c.check(
        instant(&Some(picker.dateValue()))
            == Some(ns_date(CivilDateTime::date(2026, 9, 10)).timeIntervalSinceReferenceDate()),
        "SetValue goes through the native picker",
    );
    c.check(
        rig.perform(w, "date", SemanticAction::SetValue("not a date".into())).is_err(),
        "SetValue with junk is refused",
    );
    c.check(
        rig.node(w, "date").is_some_and(|n| n.value.as_deref() == Some("2026-09-10")),
        "date node value follows the picker",
    );

    rig.update(opened, |v, cx| {
        v.disabled = true;
        cx.notify();
    });
    rig.pump();
    c.check(
        !picker.isEnabled() && rig.node(w, "date").is_some_and(|n| !n.enabled),
        "a disabled date picker is disabled natively and in the tree",
    );
    c.check(
        rig.perform(w, "date", SemanticAction::SetValue("2026-10-10".into())).is_err(),
        "a disabled date picker refuses SetValue",
    );
    rig.update(opened, |v, cx| {
        v.disabled = false;
        cx.notify();
    });
    rig.pump();
}

fn colour(rig: &Rig, c: &mut Checks, opened: &Opened, w: WindowId) {
    let Some(well) = as_class::<NSColorWell>(rig.view(w, "color")) else {
        return c.check(false, "colour well mounts as NSColorWell");
    };
    c.check(true, "colour well mounts as NSColorWell");
    c.check(well.colorWellStyle() == NSColorWellStyle::Minimal, "colour well uses the minimal style");
    c.check(
        rgba(&well.color()).is_some_and(|x| near(x, hsla_rgba(Hsla::from(rgb(0x3366ff))))),
        "colour reaches the well",
    );
    c.check(well.supportsAlpha(), "alpha reaches supportsAlpha");
    c.check(well.toolTip().is_some_and(|t| t.to_string() == "Pick a tint"), "colour tooltip reaches the view");
    c.check(
        well.accessibilityLabel().is_some_and(|t| t.to_string() == "Tint"),
        "colour accessibility label reaches the view",
    );
    c.check(
        rig.node(w, "color").is_some_and(|n| n.role == Role::ColorWell && n.value.as_deref() == Some("#3366ff")),
        "colour node has role and hex value",
    );
    let first_events = rig.read(opened, |v| v.color_events.len());
    c.check(first_events == 0, "mounting a colour emits nothing");

    let red = Hsla::from(rgb(0xff0000));
    well.setColor(&NSColor::colorWithSRGBRed_green_blue_alpha(1., 0., 0., 1.));
    user_change(&well);
    rig.pump();
    c.check(
        rig.read(opened, |v| v.color_events.clone()).len() == 1
            && rig.read(opened, |v| v.color_events[0]).to_rgb().r > 0.99,
        "a user colour change arrives as ColorChanged once",
    );
    let before = well.color();
    rig.pump();
    rig.update(opened, |_, cx| cx.notify());
    rig.pump();
    c.check(
        same_object(&before, &well.color()) && rig.read(opened, |v| v.color_events.len()) == 1,
        "feeding the reported colour back sets nothing",
    );
    c.check(rig.read(opened, |v| hsla_close(v.color, red)), "the app holds the reported colour");

    // Normalisation: without alpha the well holds an opaque colour.
    rig.update(opened, |v, cx| {
        v.alpha = false;
        v.color = Hsla::from(rgb(0x3366ff)).opacity(0.5);
        cx.notify();
    });
    rig.pump();
    rig.pump();
    let events = rig.read(opened, |v| v.color_events.clone());
    c.check(
        events.len() == 2 && (events[1].a - 1.).abs() < 0.01 && hsla_close(events[1], Hsla::from(rgb(0x3366ff))),
        "a colour the well cannot hold emits exactly one normalised event",
    );
    c.check(!well.supportsAlpha(), "alpha off reaches supportsAlpha");
    let held = well.color();
    rig.pump();
    c.check(
        same_object(&held, &well.color()) && rig.read(opened, |v| v.color_events.len()) == 2,
        "the normalised colour fed back sets nothing",
    );

    // Reentrancy: the handler writes a different colour into the controlled prop.
    rig.update(opened, |v, _| v.rewrite_color = true);
    well.setColor(&NSColor::colorWithSRGBRed_green_blue_alpha(0., 0., 1., 1.));
    user_change(&well);
    rig.pump();
    rig.pump();
    c.check(
        rig.read(opened, |v| v.color_events.len()) == 3,
        "a colour handler that sets the value emits once, no loop",
    );
    c.check(
        rgba(&well.color()).is_some_and(|x| near(x, [0., 1., 0., 1.])),
        "the colour the handler wrote reaches the well",
    );
    rig.update(opened, |v, _| v.rewrite_color = false);

    let result = rig.perform(w, "color", SemanticAction::SetValue("#ff8000".into()));
    c.check(result.is_ok(), "SetValue on the colour well succeeds");
    c.check(
        rig.read(opened, |v| v.color_events.last().copied()).is_some_and(|x| hsla_close(x, Hsla::from(rgb(0xff8000)))),
        "SetValue reaches the app as ColorChanged",
    );
    c.check(rgba(&well.color()).is_some_and(|x| near(x, [1., 0.5, 0., 1.])), "SetValue goes through the native well");
    c.check(
        rig.perform(w, "color", SemanticAction::SetValue("orange".into())).is_err(),
        "SetValue with junk is refused",
    );
    c.check(
        rig.node(w, "color").is_some_and(|n| n.value.as_deref() == Some("#ff8000")),
        "colour node value follows the well",
    );
}

fn progress_indicators(rig: &Rig, c: &mut Checks, w: WindowId) {
    let bar = as_class::<NSProgressIndicator>(rig.view(w, "bar"));
    c.check(bar.is_some(), "progress mounts as NSProgressIndicator");
    if let Some(bar) = bar {
        c.check(
            bar.style() == NSProgressIndicatorStyle::Bar
                && !bar.isIndeterminate()
                && (bar.doubleValue() - 40.).abs() < 0.01,
            "a determinate bar shows value times 100",
        );
        c.check(bar.toolTip().is_some_and(|t| t.to_string() == "Downloading"), "progress tooltip reaches the view");
        c.check(
            bar.accessibilityLabel().is_some_and(|t| t.to_string() == "Download"),
            "progress label reaches the view",
        );
        c.check(
            rig.node(w, "bar").is_some_and(|n| n.role == Role::ProgressIndicator && n.value.as_deref() == Some("40%")),
            "progress node has role and percent value",
        );
    }
    if let Some(i) = as_class::<NSProgressIndicator>(rig.view(w, "bar-indeterminate")) {
        c.check(
            i.style() == NSProgressIndicatorStyle::Bar && i.isIndeterminate(),
            "a bar with no value is indeterminate",
        );
    }
    if let Some(s) = as_class::<NSProgressIndicator>(rig.view(w, "spinner")) {
        c.check(
            s.style() == NSProgressIndicatorStyle::Spinning && s.isIndeterminate(),
            "a spinner is spinning and indeterminate",
        );
        let size = rig.node(w, "spinner").and_then(|n| n.bounds).map(|b| b.size);
        c.check(size.is_some_and(|s| s.width > 0. && s.height > 0.), "the spinner is measured from AppKit");
    }
    if let Some(ci) = as_class::<NSProgressIndicator>(rig.view(w, "circular")) {
        c.check(
            ci.style() == NSProgressIndicatorStyle::Spinning
                && !ci.isIndeterminate()
                && (ci.doubleValue() - 25.).abs() < 0.01,
            "a circular progress is a determinate spinning indicator",
        );
    }
}

fn labels(rig: &Rig, c: &mut Checks, opened: &Opened, w: WindowId) {
    let mut fonts = true;
    for (i, style) in STYLES.into_iter().enumerate() {
        let Some(field) = as_class::<NSTextField>(rig.view(w, &format!("style-{i}"))) else {
            fonts = false;
            continue;
        };
        let (want, got) = (expected_font(style), field.font());
        fonts &=
            got.is_some_and(|g| g.fontName() == want.fontName() && (g.pointSize() - want.pointSize()).abs() < 0.01);
    }
    c.check(fonts, "every label text style is NSFont's preferred font for that style");
    let primary = as_class::<NSTextField>(rig.view(w, "style-0"));
    c.check(
        primary.is_some_and(|f| !f.isEditable() && f.stringValue().to_string() == "Style 0"),
        "a label is a non-editable text field",
    );
    c.check(
        as_class::<NSTextField>(rig.view(w, "style-6"))
            .is_some_and(|f| f.textColor().is_some_and(|t| *t == *NSColor::labelColor())),
        "primary tone is labelColor",
    );
    c.check(
        as_class::<NSTextField>(rig.view(w, "secondary"))
            .is_some_and(|f| f.textColor().is_some_and(|t| *t == *NSColor::secondaryLabelColor())),
        "secondary tone is secondaryLabelColor",
    );
    c.check(
        as_class::<NSTextField>(rig.view(w, "danger"))
            .is_some_and(|f| f.textColor().is_some_and(|t| *t == *NSColor::systemRedColor())),
        "danger tone is systemRedColor",
    );
    c.check(
        as_class::<NSTextField>(rig.view(w, "selectable")).is_some_and(|f| f.isSelectable()),
        "selectable reaches the field",
    );
    c.check(
        as_class::<NSTextField>(rig.view(w, "style-0")).is_some_and(|f| !f.isSelectable()),
        "a label is not selectable by default",
    );
    c.check(
        rig.node(w, "secondary").is_some_and(|n| n.role == Role::Text && n.name == "Secondary"),
        "label node has role text and its string as name",
    );
    let single = as_class::<NSTextField>(rig.view(w, "wrapping"));
    let single_height = rig.node(w, "wrapping").and_then(|n| n.bounds).map(|b| b.size.height);
    c.check(
        single.is_some_and(|f| f.lineBreakMode() == NSLineBreakMode::ByTruncatingTail),
        "a label truncates at the tail by default",
    );
    rig.update(opened, |v, cx| {
        v.wrap = true;
        cx.notify();
    });
    rig.pump();
    let wrapped = as_class::<NSTextField>(rig.view(w, "wrapping"));
    let wrapped_height = rig.node(w, "wrapping").and_then(|n| n.bounds).map(|b| b.size.height);
    c.check(
        wrapped.is_some_and(|f| f.lineBreakMode() == NSLineBreakMode::ByWordWrapping),
        "wraps selects word wrapping",
    );
    c.check(
        matches!((single_height, wrapped_height), (Some(a), Some(b)) if b > a * 1.5),
        "a wrapping label is measured with the offered width and grows taller",
    );
}

fn links(rig: &Rig, c: &mut Checks, opened: &Opened, w: WindowId) {
    let Some(button) = as_class::<NSButton>(rig.view(w, "link")) else {
        return c.check(false, "link mounts as an NSButton");
    };
    c.check(true, "link mounts as an NSButton");
    c.check(!button.isBordered(), "a link button is borderless");
    let title = button.attributedTitle();
    c.check(title.string().to_string() == "Read more", "link title reaches the button");
    let attrs: Retained<NSDictionary<_, AnyObject>> =
        unsafe { title.attributesAtIndex_effectiveRange(0, std::ptr::null_mut()) };
    let color = unsafe { attrs.objectForKey(NSForegroundColorAttributeName) };
    c.check(
        color.is_some_and(|c| c.downcast_ref::<NSColor>().is_some_and(|c| *c == *NSColor::linkColor())),
        "link title is drawn in linkColor",
    );
    c.check(unsafe { attrs.objectForKey(NSFontAttributeName) }.is_some(), "link title carries a font");
    c.check(button.toolTip().is_some_and(|t| t.to_string() == "Opens the docs"), "link tooltip reaches the view");
    c.check(
        rig.node(w, "link").is_some_and(|n| {
            n.role == Role::Link && n.name == "Read more" && n.value.as_deref() == Some("https://example.com/docs")
        }),
        "link node has role, title and url value",
    );

    let result = rig.perform(w, "link", SemanticAction::Click);
    c.check(result.is_ok(), "Click on the link succeeds");
    c.check(
        rig.read(opened, |v| v.link_clicks.clone()) == vec![Some(SharedString::from("https://example.com/docs"))],
        "Click reaches the app as LinkActivated with the url",
    );

    rig.update(opened, |v, cx| {
        v.disabled = true;
        cx.notify();
    });
    rig.pump();
    c.check(
        !button.isEnabled() && rig.node(w, "link").is_some_and(|n| !n.enabled),
        "a disabled link is disabled natively and in the tree",
    );
    c.check(rig.perform(w, "link", SemanticAction::Click).is_err(), "a disabled link refuses Click");
    c.check(rig.read(opened, |v| v.link_clicks.len()) == 1, "a disabled link emits nothing");
}
