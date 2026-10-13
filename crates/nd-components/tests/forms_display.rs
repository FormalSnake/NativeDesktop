//! Tier 1 behaviour of labels, links, progress, buttons and the settings-row layout.

mod common;

use common::*;
use nd_components::form::{FormProps, FormRowProps, FormSectionProps};
use nd_components::forms::*;
use nd_components::{ButtonExt, Icon, form, form_section};
use nd_ui::prelude::*;
use nd_ui::semantics::{ActionError, SemanticAction};
use nd_ui::{Bezel, ButtonProps, ButtonVariant, Role, Size};

#[derive(Default)]
struct Display {
    fraction: f32,
    clicked: Vec<Option<String>>,
}

fn display_view(s: &Display, cx: &mut Context<Host<Display>>) -> AnyElement {
    v_flex()
        .child(label("Inbox").id("title").text_style(TextStyle::LargeTitle))
        .child(label("3 unread").id("caption").text_style(TextStyle::Caption1).secondary())
        .child(label("Could not connect").id("error").tone(LabelTone::Danger).selectable().wraps())
        .child(link("docs", "Read the docs").url("https://example.com/docs").on_click(cx.listener(
            |h, e: &LinkActivated, _, cx| {
                h.s.clicked.push(e.url.as_ref().map(|u| u.to_string()));
                cx.notify();
            },
        )))
        .child(link("inline", "Show details").keep_internal().on_click(cx.listener(|h, e: &LinkActivated, _, cx| {
            h.s.clicked.push(e.url.as_ref().map(|u| u.to_string()));
            cx.notify();
        })))
        .child(link("dead", "Unavailable").disabled(true))
        .child(progress(Some(s.fraction)).id("bar").label("Downloading"))
        .child(progress(None).id("busy").label("Preparing"))
        .child(spinner().id("spin").label("Loading"))
        .child(progress(Some(0.25)).id("ring").circular().label("Quota"))
        .into_any_element()
}

#[test]
fn labels_carry_text_style_tone_and_a_text_node() {
    let rig = Rig::new(Display::default(), display_view);
    assert!(rig.kind_is::<LabelKind>("title"));
    let title = rig.props::<LabelProps>("title");
    assert_eq!((title.text_style, title.tone), (TextStyle::LargeTitle, LabelTone::Primary));
    let caption = rig.props::<LabelProps>("caption");
    assert_eq!((caption.text_style, caption.tone), (TextStyle::Caption1, LabelTone::Secondary));
    let error = rig.props::<LabelProps>("error");
    assert_eq!((error.tone, error.selectable, error.wraps), (LabelTone::Danger, true, true));
    let node = rig.sem("title");
    assert_eq!((node.role, node.name.as_str()), (Role::Text, "Inbox"));
}

#[test]
fn labels_take_no_actions() {
    let mut rig = Rig::new(Display::default(), display_view);
    assert!(matches!(rig.perform("title", SemanticAction::Click), Err(ActionError::Unsupported(_))));
    assert!(matches!(rig.set_value("title", "x"), Err(ActionError::Unsupported(_))));
}

#[test]
fn link_click_fires_activation_with_its_url() {
    let mut rig = Rig::new(Display::default(), display_view);
    assert!(rig.kind_is::<LinkKind>("docs"));
    let node = rig.sem("docs");
    assert_eq!(
        (node.role, node.name.as_str(), node.value.as_deref()),
        (Role::Link, "Read the docs", Some("https://example.com/docs"))
    );
    assert!(rig.props::<LinkProps>("docs").open_external);
    assert!(!rig.props::<LinkProps>("inline").open_external);

    rig.perform("docs", SemanticAction::Click).unwrap();
    rig.perform("inline", SemanticAction::Click).unwrap();
    assert_eq!(rig.state().clicked, [Some("https://example.com/docs".to_owned()), None]);
    rig.emit("docs", LinkActivated { url: None });
    assert_eq!(rig.state().clicked.len(), 3);
}

#[test]
fn disabled_link_refuses() {
    let mut rig = Rig::new(Display::default(), display_view);
    assert!(!rig.sem("dead").enabled);
    assert!(refused(rig.perform("dead", SemanticAction::Click)));
    assert!(rig.state().clicked.is_empty());
    assert!(matches!(rig.set_value("docs", "x"), Err(ActionError::Unsupported(_))));
}

#[test]
fn progress_reads_as_a_percentage_and_updates_once_per_change() {
    let mut rig = Rig::new(Display { fraction: 0.4, ..Display::default() }, display_view);
    assert!(rig.kind_is::<ProgressKind>("bar"));
    let node = rig.sem("bar");
    assert_eq!(
        (node.role, node.name.as_str(), node.value.as_deref()),
        (Role::ProgressIndicator, "Downloading", Some("40%"))
    );
    assert_eq!(rig.props::<ProgressProps>("bar").presentation, ProgressStyle::Bar);

    rig.fresh();
    rig.mutate(|s| s.fraction = 0.75);
    assert_eq!(rig.updates("bar"), 1);
    assert_eq!(rig.all_updates(), 1, "nothing else changed");
    assert_eq!(rig.sem("bar").value.as_deref(), Some("75%"));
    rig.mutate(|s| s.fraction = 1.4);
    assert_eq!(rig.sem("bar").value.as_deref(), Some("100%"), "clamped");
}

#[test]
fn indeterminate_spinner_and_circular_presentations() {
    let rig = Rig::new(Display::default(), display_view);
    assert_eq!(rig.sem("busy").value, None, "indeterminate has no fraction");
    assert_eq!(rig.props::<ProgressProps>("busy").value, None);
    let spin = rig.props::<ProgressProps>("spin");
    assert_eq!((spin.presentation, spin.value), (ProgressStyle::Spinner, None));
    assert_eq!(rig.sem("spin").role, Role::ProgressIndicator);
    assert_eq!(rig.props::<ProgressProps>("ring").presentation, ProgressStyle::Circular);
    assert_eq!(rig.sem("ring").value.as_deref(), Some("25%"));
}

#[derive(Default)]
struct Buttons {
    clicks: u32,
}

#[test]
fn button_styles_map_to_variant_and_bezel() {
    fn view(_: &Buttons, cx: &mut Context<Host<Buttons>>) -> AnyElement {
        let click = cx.listener(|h, _: &ClickEvent, _, cx| {
            h.s.clicks += 1;
            cx.notify();
        });
        v_flex()
            .child(button("push", "Push").on_click(click))
            .child(button("primary", "Primary").primary())
            .child(button("prominent", "Prominent").prominent())
            .child(button("borderless", "Borderless").borderless())
            .child(button("toolbar", "Toolbar").toolbar_style())
            .child(button("destructive", "Delete").destructive())
            .into_any_element()
    }
    let mut rig = Rig::new(Buttons::default(), view);
    let of = |id: &str| {
        let p = rig.props::<ButtonProps>(id);
        (p.variant, p.bezel)
    };
    assert_eq!(of("push"), (ButtonVariant::Standard, Bezel::Automatic));
    assert_eq!(of("primary").0, ButtonVariant::Primary);
    assert_eq!(of("prominent").0, ButtonVariant::Accent);
    assert_eq!(of("borderless").1, Bezel::Borderless);
    assert_eq!(of("toolbar").1, Bezel::Toolbar);
    assert_eq!(of("destructive").0, ButtonVariant::Destructive);
    rig.perform("push", SemanticAction::Click).unwrap();
    assert_eq!(rig.state().clicks, 1);
}

#[derive(Default)]
struct Rows {
    airplane: bool,
    opened: u32,
    error: bool,
}

fn rows_view(s: &Rows, cx: &mut Context<Host<Rows>>) -> AnyElement {
    form()
        .child(settings_group("Network").child(
            switch_row("airplane", "Airplane mode").subtitle("Turns off all radios").checked(s.airplane).on_toggle(
                cx.listener(|h, e: &Toggled, _, cx| {
                    h.s.airplane = e.checked;
                    cx.notify();
                }),
            ),
        ))
        .child(
            form_section()
                .title("General")
                .footer("Changes apply to every window.")
                .child(navigation_row(
                    "about",
                    "About",
                    cx.listener(|h, _: &RowActivated, _, cx| {
                        h.s.opened += 1;
                        cx.notify();
                    }),
                ))
                .child(row("Volume").id("volume-row").child(slider("volume", 0.5).label("Volume"))),
        )
        .into_any_element()
}

#[test]
fn form_sections_and_rows_carry_their_text_and_hold_one_control() {
    let rig = Rig::new(Rows::default(), rows_view);
    assert_eq!(rig.find_props::<FormProps>().len(), 1);
    let sections: Vec<_> = rig
        .find_props::<FormSectionProps>()
        .into_iter()
        .map(|n| rig.cx.backend().props(n).unwrap().downcast_ref::<FormSectionProps>().unwrap().clone())
        .collect();
    assert_eq!(sections[0].title.as_deref(), Some("Network"));
    assert_eq!(sections[1].title.as_deref(), Some("General"));
    assert_eq!(sections[1].footer.as_deref(), Some("Changes apply to every window."));

    let backend = rig.cx.backend();
    let volume = rig.node("volume");
    let row_node = backend.parent(volume).expect("the control sits in a row");
    let row = backend.props(row_node).unwrap().downcast_ref::<FormRowProps>().unwrap();
    assert_eq!(row.label.as_ref(), "Volume");
    assert_eq!(backend.children(row_node), [volume]);
    let section = backend.parent(row_node).unwrap();
    assert!(backend.props(section).unwrap().downcast_ref::<FormSectionProps>().is_some());
}

#[test]
fn switch_row_names_its_switch_with_the_title_and_toggles() {
    let mut rig = Rig::new(Rows::default(), rows_view);
    let row_node = rig.node("airplane-row");
    let row = rig.cx.backend().props(row_node).unwrap().downcast_ref::<FormRowProps>().unwrap().clone();
    assert_eq!((row.label.as_ref(), row.help.as_deref()), ("Airplane mode", Some("Turns off all radios")));
    let node = rig.sem("airplane");
    assert_eq!((node.role, node.name.as_str(), node.checked), (Role::Switch, "Airplane mode", Some(false)));
    assert_eq!(rig.cx.backend().parent(rig.node("airplane")), Some(row_node));

    rig.fresh();
    rig.perform("airplane", SemanticAction::Click).unwrap();
    assert!(rig.state().airplane);
    assert_eq!(rig.updates("airplane"), 1);
    assert_eq!(rig.sem("airplane").checked, Some(true));
}

#[test]
fn disabled_switch_row_refuses() {
    fn view(_: &Rows, _: &mut Context<Host<Rows>>) -> AnyElement {
        switch_row("locked", "Managed by your administrator").disabled(true).into_any_element()
    }
    let mut rig = Rig::new(Rows::default(), view);
    assert!(!rig.sem("locked").enabled);
    assert!(refused(rig.perform("locked", SemanticAction::Click)));
}

#[test]
fn navigation_row_is_a_chevron_button_that_reports_activation() {
    let mut rig = Rig::new(Rows::default(), rows_view);
    let props = rig.props::<ButtonProps>("about");
    assert_eq!((props.icon.is_some(), props.bezel, props.tooltip.as_deref()), (true, Bezel::Borderless, Some("About")));
    assert_eq!(props.icon.as_deref(), Some(Icon::ChevronRight.lucide()));
    rig.perform("about", SemanticAction::Click).unwrap();
    rig.perform("about", SemanticAction::Click).unwrap();
    assert_eq!(rig.state().opened, 2);
}

fn field_view(s: &Rows, _: &mut Context<Host<Rows>>) -> AnyElement {
    form_field("Email")
        .help("Used for receipts")
        .error(s.error.then_some("Enter an address like name@example.com"))
        .control(text_field("email").placeholder("name@example.com").label("Email"))
        .into_any_element()
}

fn labels_of(rig: &Rig<Rows>) -> Vec<LabelProps> {
    rig.find_props::<LabelProps>()
        .into_iter()
        .map(|n| rig.cx.backend().props(n).unwrap().downcast_ref::<LabelProps>().unwrap().clone())
        .collect()
}

#[test]
fn form_field_shows_help_until_an_error_replaces_it() {
    let mut rig = Rig::new(Rows::default(), field_view);
    let labels = labels_of(&rig);
    let texts: Vec<_> = labels.iter().map(|l| (l.text.as_ref(), l.text_style, l.tone)).collect();
    assert_eq!(
        texts,
        [
            ("Email", TextStyle::Headline, LabelTone::Primary),
            ("Used for receipts", TextStyle::Footnote, LabelTone::Secondary)
        ]
    );
    assert_eq!(rig.sem("email").placeholder.as_deref(), Some("name@example.com"));

    rig.mutate(|s| s.error = true);
    let labels = labels_of(&rig);
    let texts: Vec<_> = labels.iter().map(|l| (l.text.as_ref(), l.tone)).collect();
    assert_eq!(texts, [("Email", LabelTone::Primary), ("Enter an address like name@example.com", LabelTone::Danger)]);

    rig.mutate(|s| s.error = false);
    assert_eq!(labels_of(&rig).len(), 2);
    assert_eq!(labels_of(&rig)[1].text.as_ref(), "Used for receipts");
}

#[test]
fn clamp_caps_its_width_and_centres_in_wide_windows() {
    fn view(_: &Rows, _: &mut Context<Host<Rows>>) -> AnyElement {
        clamp().child(div().id("inner").h(px(20.))).into_any_element()
    }
    let wide = Rig::sized(Size { width: 900., height: 300. }, |_| Rows::default(), view);
    let bounds = wide.sem("inner").bounds.expect("laid out");
    assert_eq!((bounds.origin.x, bounds.size.width), (150., 600.));
    let narrow = Rig::sized(Size { width: 400., height: 300. }, |_| Rows::default(), view);
    let bounds = narrow.sem("inner").bounds.expect("laid out");
    assert_eq!((bounds.origin.x, bounds.size.width), (0., 400.));
}

#[test]
fn clamp_maximum_overrides_the_default_width() {
    fn view(_: &Rows, _: &mut Context<Host<Rows>>) -> AnyElement {
        clamp().maximum(px(300.)).child(div().id("inner").h(px(20.))).into_any_element()
    }
    let rig = Rig::sized(Size { width: 900., height: 300. }, |_| Rows::default(), view);
    assert_eq!(rig.sem("inner").bounds.unwrap().size.width, 300.);
}

#[test]
fn button_group_keeps_its_buttons_in_order() {
    fn view(_: &Rows, _: &mut Context<Host<Rows>>) -> AnyElement {
        button_group().child(button("left", "Left")).child(button("right", "Right")).into_any_element()
    }
    let rig = Rig::new(Rows::default(), view);
    let (left, right) = (rig.node("left"), rig.node("right"));
    let parent = rig.cx.backend().parent(left).unwrap();
    assert_eq!(rig.cx.backend().children(parent), [left, right]);
}
