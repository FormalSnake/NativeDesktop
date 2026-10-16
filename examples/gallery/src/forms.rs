use nd::prelude::*;

/// Runs `$body` on the page when the control reports `$ty`, then renders again.
macro_rules! on {
    ($cx:ident, $ty:ty, |$page:ident, $e:ident| $body:expr) => {
        $cx.listener(|$page, $e: &$ty, _, cx| {
            $body;
            cx.notify();
        })
    };
}

const TEXT_STYLES: [(&str, TextStyle); 11] = [
    ("Large title", TextStyle::LargeTitle),
    ("Title 1", TextStyle::Title1),
    ("Title 2", TextStyle::Title2),
    ("Title 3", TextStyle::Title3),
    ("Headline", TextStyle::Headline),
    ("Subheadline", TextStyle::Subheadline),
    ("Body", TextStyle::Body),
    ("Callout", TextStyle::Callout),
    ("Footnote", TextStyle::Footnote),
    ("Caption 1", TextStyle::Caption1),
    ("Caption 2", TextStyle::Caption2),
];

pub struct FormsPage {
    action: SharedString,
    name: String,
    email: String,
    password: String,
    notes: String,
    zone: String,
    search: Entity<TextFieldState>,
    attachments: bool,
    copy_photos: bool,
    copy_videos: bool,
    wifi: bool,
    bluetooth: bool,
    appearance: usize,
    range: usize,
    sort: usize,
    volume: f32,
    brightness: f32,
    copies: f64,
    day: f64,
    due: CivilDateTime,
    alarm: CivilDateTime,
    event: CivilDateTime,
    tint: Hsla,
    download: f32,
    signup: String,
}

impl FormsPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let search = cx.new(|_| TextFieldState::default().placeholder("Search settings"));
        Self {
            action: "None yet".into(),
            name: String::new(),
            email: String::new(),
            password: String::new(),
            notes: String::new(),
            zone: String::new(),
            search,
            attachments: true,
            copy_photos: true,
            copy_videos: false,
            wifi: true,
            bluetooth: false,
            appearance: 2,
            range: 1,
            sort: 0,
            volume: 40.,
            brightness: 0.6,
            copies: 1.,
            day: 1.,
            due: CivilDateTime::date(2026, 10, 13),
            alarm: CivilDateTime::date(2026, 10, 13).at(7, 30, 0),
            event: CivilDateTime::date(2026, 10, 13).at(18, 0, 0),
            tint: hsla(0.6, 0.8, 0.5, 1.),
            download: 0.25,
            signup: String::new(),
        }
    }

    pub fn action(&self) -> &SharedString {
        &self.action
    }

    fn did(&mut self, what: &str) {
        self.action = what.to_owned().into();
    }

    fn text(&self, cx: &mut Context<Self>) -> FormSection {
        let search = self.search.read(cx).text().clone();
        let searching =
            if search.is_empty() { "Matches setting names".into() } else { format!("Searching for {search}") };
        form_section()
            .title("Text")
            .child(form_row("Last action").child(label(self.action.clone()).id("action").secondary()))
            .child(
                form_row("Name").child(
                    text_field("name")
                        .text(self.name.clone())
                        .placeholder("Full name")
                        .label("Name")
                        .w(px(220.))
                        .on_change(on!(cx, TextChanged, |p, e| p.name = e.text.to_string())),
                ),
            )
            .child(
                form_row("Email").child(
                    text_field("email")
                        .text(self.email.clone())
                        .placeholder("name@example.com")
                        .label("Email")
                        .leading_icon(Icon::Mail, "Email address")
                        .w(px(220.))
                        .on_change(on!(cx, TextChanged, |p, e| p.email = e.text.to_string())),
                ),
            )
            .child(
                form_row("Password").child(
                    secure_field("password")
                        .text(self.password.clone())
                        .placeholder("Required")
                        .label("Password")
                        .leading_icon(Icon::Lock, "Password")
                        .w(px(220.))
                        .on_change(on!(cx, TextChanged, |p, e| p.password = e.text.to_string())),
                ),
            )
            .child(
                form_row("Search")
                    .help(searching)
                    .child(search_field("search", &self.search).label("Search").w(px(220.))),
            )
            .child(
                form_row("Notes").child(
                    text_area("notes")
                        .text(self.notes.clone())
                        .placeholder("Anything worth remembering")
                        .label("Notes")
                        .min_lines(4)
                        .w(px(260.))
                        .on_change(on!(cx, TextChanged, |p, e| p.notes = e.text.to_string())),
                ),
            )
            .child(
                form_row("Time zone").child(
                    combo_box("zone", ["UTC", "Europe/Madrid", "Atlantic/Canary", "America/New_York"])
                        .text(self.zone.clone())
                        .placeholder("Choose or type")
                        .label("Time zone")
                        .w(px(220.))
                        .on_change(on!(cx, TextChanged, |p, e| p.zone = e.text.to_string())),
                ),
            )
    }

    fn buttons(&self, cx: &mut Context<Self>) -> FormSection {
        form_section()
            .title("Buttons")
            .child(form_row("Push").child(
                button("push", "Save draft").on_click(on!(cx, ClickEvent, |p, _e| p.did("Pushed the push button"))),
            ))
            .child(
                form_row("Default action").child(button("primary", "Continue").primary().on_click(on!(
                    cx,
                    ClickEvent,
                    |p, _e| p.did("Pushed the default button")
                ))),
            )
            .child(
                form_row("Prominent").child(button("prominent", "Upgrade").prominent().on_click(on!(
                    cx,
                    ClickEvent,
                    |p, _e| p.did("Pushed the prominent button")
                ))),
            )
            .child(
                form_row("Borderless").child(button("borderless", "Skip for now").borderless().on_click(on!(
                    cx,
                    ClickEvent,
                    |p, _e| p.did("Pushed the borderless button")
                ))),
            )
            .child(form_row("Toolbar style").child(
                button("toolbar-style", "Share").icon(Icon::Share).toolbar_style().on_click(on!(
                    cx,
                    ClickEvent,
                    |p, _e| p.did("Pushed the toolbar button")
                )),
            ))
            .child(form_row("Destructive").child(button("destructive", "Delete account").destructive().on_click(on!(
                cx,
                ClickEvent,
                |p, _e| p.did("Pushed the destructive button")
            ))))
    }

    fn toggles(&self, cx: &mut Context<Self>) -> FormSection {
        let both = self.copy_photos && self.copy_videos;
        form_section()
            .title("Toggles")
            .child(form_row("Attachments").child(
                checkbox("attachments", "Include attachments").checked(self.attachments).on_toggle(on!(
                    cx,
                    Toggled,
                    |p, e| p.attachments = e.checked
                )),
            ))
            .child(
                form_row("Back up").child(
                    v_flex()
                        .gap_1()
                        .child(
                            checkbox("copy-all", "Everything")
                                .checked(both)
                                .mixed(self.copy_photos != self.copy_videos)
                                .on_toggle(on!(cx, Toggled, |p, e| {
                                    p.copy_photos = e.checked;
                                    p.copy_videos = e.checked;
                                })),
                        )
                        .child(checkbox("copy-photos", "Photos").checked(self.copy_photos).on_toggle(on!(
                            cx,
                            Toggled,
                            |p, e| p.copy_photos = e.checked
                        )))
                        .child(checkbox("copy-videos", "Videos").checked(self.copy_videos).on_toggle(on!(
                            cx,
                            Toggled,
                            |p, e| p.copy_videos = e.checked
                        ))),
                ),
            )
            .child(form_row("Wi-Fi").child(switch("wifi").label("Wi-Fi").checked(self.wifi).on_toggle(on!(
                cx,
                Toggled,
                |p, e| p.wifi = e.checked
            ))))
            .child(
                switch_row("bluetooth", "Bluetooth")
                    .subtitle("Connect to accessories nearby")
                    .checked(self.bluetooth)
                    .on_toggle(on!(cx, Toggled, |p, e| p.bluetooth = e.checked)),
            )
    }

    fn choices(&self, cx: &mut Context<Self>) -> FormSection {
        form_section()
            .title("Choices")
            .child(
                form_row("Appearance").child(
                    radio_group("appearance", ["Light", "Dark", "Automatic"])
                        .selected(self.appearance)
                        .label("Appearance")
                        .on_change(on!(cx, SelectChanged, |p, e| p.appearance = e.index)),
                ),
            )
            .child(form_row("Show").child(
                segmented("range", ["Day", "Week", "Month"]).selected(self.range).label("Show").on_change(on!(
                    cx,
                    SelectChanged,
                    |p, e| p.range = e.index
                )),
            ))
            .child(form_row("Sort by").child(
                select("sort", ["Name", "Date modified", "Size"]).selected(self.sort).label("Sort by").on_change(on!(
                    cx,
                    SelectChanged,
                    |p, e| p.sort = e.index
                )),
            ))
    }

    fn values(&self, cx: &mut Context<Self>) -> FormSection {
        form_section()
            .title("Values")
            .child(
                form_row("Volume").help(format!("{:.0} percent", self.volume)).child(
                    slider("volume", self.volume)
                        .range(0.0..=100.0)
                        .step(10.)
                        .ticks(11)
                        .label("Volume")
                        .w(px(220.))
                        .on_change(on!(cx, SliderChanged, |p, e| p.volume = e.value)),
                ),
            )
            .child(form_row("Brightness").child(
                slider("brightness", self.brightness).range(0.0..=1.0).label("Brightness").w(px(220.)).on_change(on!(
                    cx,
                    SliderChanged,
                    |p, e| p.brightness = e.value
                )),
            ))
            .child(form_row("Copies").child(
                stepper("copies", self.copies).range(1.0..=20.0).with_field(0).label("Copies").on_change(on!(
                    cx,
                    StepperChanged,
                    |p, e| p.copies = e.value
                )),
            ))
            .child(form_row("Day of the week").help("Goes round after Sunday").child(
                stepper("day", self.day).range(1.0..=7.0).wraps(true).label("Day of the week").on_change(on!(
                    cx,
                    StepperChanged,
                    |p, e| p.day = e.value
                )),
            ))
    }

    fn dates(&self, cx: &mut Context<Self>) -> FormSection {
        form_section()
            .title("Date, time and colour")
            .child(form_row("Due date").child(
                date_picker("due", self.due).label("Due date").min(CivilDateTime::date(2026, 1, 1)).on_change(on!(
                    cx,
                    DateChanged,
                    |p, e| p.due = e.value
                )),
            ))
            .child(
                form_row("Alarm").child(
                    date_picker("alarm", self.alarm)
                        .mode(DatePickerMode::Time)
                        .presentation(DatePickerStyle::Stepper)
                        .label("Alarm")
                        .on_change(on!(cx, DateChanged, |p, e| p.alarm = e.value)),
                ),
            )
            .child(
                form_row("Event").child(
                    date_picker("event", self.event)
                        .mode(DatePickerMode::DateTime)
                        .presentation(DatePickerStyle::Graphical)
                        .label("Event")
                        .on_change(on!(cx, DateChanged, |p, e| p.event = e.value)),
                ),
            )
            .child(form_row("Accent colour").child(
                color_well("tint", self.tint).alpha(true).label("Accent colour").on_change(on!(
                    cx,
                    ColorChanged,
                    |p, e| p.tint = e.color
                )),
            ))
    }

    fn progress(&self, cx: &mut Context<Self>) -> FormSection {
        form_section()
            .title("Progress")
            .child(
                form_row("Download").child(progress(Some(self.download)).id("download").label("Download").w(px(220.))),
            )
            .child(form_row("Advance download").child(button("advance", "Advance").on_click(cx.listener(
                |p, _: &ClickEvent, _, cx| {
                    p.download = if p.download >= 1. { 0. } else { (p.download + 0.25).min(1.) };
                    cx.notify();
                },
            ))))
            .child(form_row("Preparing").child(progress(None).id("preparing").label("Preparing").w(px(220.))))
            .child(form_row("Loading").child(spinner().id("loading").label("Loading")))
            .child(form_row("Storage used").child(progress(Some(0.4)).id("storage").circular().label("Storage used")))
    }

    fn text_styles(&self) -> FormSection {
        let styles = TEXT_STYLES.iter().map(|(name, style)| {
            form_row(*name).child(label("The quick brown fox").id(format!("style-{name}")).text_style(*style))
        });
        form_section()
            .title("Text styles")
            .children(styles)
            .child(form_row("Primary").child(label("Primary text").id("tone-primary")))
            .child(form_row("Secondary").child(label("Supporting text").id("tone-secondary").secondary()))
            .child(form_row("Danger").child(label("Could not connect").id("tone-danger").tone(LabelTone::Danger)))
    }

    fn rows(&self, cx: &mut Context<Self>) -> FormSection {
        let error =
            (!self.signup.is_empty() && !self.signup.contains('@')).then_some("Enter an address like name@example.com");
        form_section()
            .title("Rows and layout")
            .footer("Rows take System Settings metrics on macOS and the skin's elsewhere.")
            .child(form_row("Documentation").child(link("docs", "Read about forms").keep_internal().on_click(on!(
                cx,
                LinkActivated,
                |p, _e| p.did("Activated the link")
            ))))
            .child(navigation_row("about", "About this Mac", on!(cx, RowActivated, |p, _e| p.did("Opened About"))))
            .child(
                form_row("Sign up").child(
                    clamp()
                        .maximum(px(360.))
                        .child(
                            form_field("Email").help("Used for receipts").error(error).control(
                                text_field("signup")
                                    .text(self.signup.clone())
                                    .placeholder("name@example.com")
                                    .label("Email")
                                    .on_change(on!(cx, TextChanged, |p, e| p.signup = e.text.to_string())),
                            ),
                        )
                        .child(
                            button_group()
                                .child(
                                    button("signup-cancel", "Cancel")
                                        .on_click(on!(cx, ClickEvent, |p, _e| p.did("Cancelled"))),
                                )
                                .child(button("signup-create", "Create account").primary().on_click(on!(
                                    cx,
                                    ClickEvent,
                                    |p, _e| p.did("Created an account")
                                ))),
                        ),
                ),
            )
    }
}

impl Render for FormsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        form()
            .title("Forms")
            .child(self.text(cx))
            .child(self.buttons(cx))
            .child(self.toggles(cx))
            .child(self.choices(cx))
            .child(self.values(cx))
            .child(self.dates(cx))
            .child(self.progress(cx))
            .child(self.text_styles())
            .child(self.rows(cx))
    }
}
