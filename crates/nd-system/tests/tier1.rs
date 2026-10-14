//! Tier 1: every service through the fake backend, plus the parts that are pure logic (the
//! event bus, click data, dialog scripts, command-line opens, directories).

use std::path::PathBuf;
use std::time::Duration;

use futures_executor::block_on;
use nd_core::WindowId;
use nd_system::fake::Call;
use nd_system::{
    AudioEvent, AudioState, ClipboardImage, DialogScript, Error, Feature, MediaCommand, MessageOptions, Notification,
    NowPlaying, OpenOptions, PickKind, PlayOptions, PlaybackState, SaveOptions, System, SystemEvent, Tray, TrayIcon,
    TrayId, TrayMenuItem,
};

#[test]
fn dialogs_answer_and_cancel() {
    let (sys, fake) = System::fake();
    fake.answer_open(Some(vec!["/tmp/a.txt".into(), "/tmp/b.txt".into()]));
    let picked = block_on(sys.dialogs().open(OpenOptions::file().multiple().filter("Text", &["txt"]))).unwrap();
    assert_eq!(picked, Some(vec![PathBuf::from("/tmp/a.txt"), PathBuf::from("/tmp/b.txt")]));
    assert_eq!(block_on(sys.dialogs().open(OpenOptions::folder())).unwrap(), None, "nothing queued is a cancel");

    fake.answer_save(Some("/tmp/out.md".into()));
    let saved = block_on(sys.dialogs().save(SaveOptions::new().file_name("out.md"))).unwrap();
    assert_eq!(saved, Some(PathBuf::from("/tmp/out.md")));

    fake.answer_message(1);
    let alert = MessageOptions::new("Discard?").button("Discard").button("Cancel");
    assert_eq!(block_on(sys.dialogs().message(alert.clone())).unwrap(), 1);
    assert_eq!(block_on(sys.dialogs().message(alert)).unwrap(), 1, "dismissed is the last button");

    let calls = fake.calls();
    assert!(matches!(&calls[0], Call::Open(o) if o.multiple && o.filters[0].extensions == ["txt"]));
    assert!(matches!(&calls[1], Call::Open(o) if o.pick == PickKind::Folders));
    assert!(matches!(&calls[2], Call::Save(s) if s.file_name.as_deref() == Some("out.md")));
}

#[test]
fn alerts_hold_at_most_three_buttons() {
    let (sys, fake) = System::fake();
    let alert = MessageOptions::new("Too many").button("A").button("B").button("C").button("D");
    assert!(matches!(block_on(sys.dialogs().message(alert)), Err(Error::Invalid(_))));
    assert!(fake.calls().is_empty(), "the backend never saw it");
}

#[test]
fn dialog_scripts_answer_in_order_and_fail_loudly_when_dry() {
    let script = DialogScript::parse(
        r#"{ "dialog.openFile": [{ "paths": ["/tmp/a"] }, { "paths": [] }],
             "dialog.showMessage": [{ "button": 2 }] }"#,
    )
    .unwrap();
    let (sys, fake) = System::fake();
    let sys = sys.with_dialog_script(Some(script));
    assert_eq!(block_on(sys.dialogs().open(OpenOptions::file())).unwrap(), Some(vec![PathBuf::from("/tmp/a")]));
    assert_eq!(block_on(sys.dialogs().open(OpenOptions::file())).unwrap(), None);
    assert!(
        matches!(block_on(sys.dialogs().open(OpenOptions::file())), Err(Error::Invalid(m)) if m.contains("no answers left"))
    );
    assert_eq!(block_on(sys.dialogs().message(MessageOptions::new("x"))).unwrap(), 2);
    // A method the script does not mention shows the real dialog.
    fake.answer_save(Some("/real".into()));
    assert_eq!(block_on(sys.dialogs().save(SaveOptions::new())).unwrap(), Some(PathBuf::from("/real")));
    assert!(matches!(fake.calls().as_slice(), [Call::Save(_)]));
    assert!(DialogScript::parse("[1, 2]").is_err());
}

#[test]
fn clipboard_holds_one_kind_at_a_time() {
    let (sys, _fake) = System::fake();
    let clip = sys.clipboard();
    assert_eq!(block_on(clip.read_text()).unwrap(), None);
    block_on(clip.write_text("hello")).unwrap();
    assert_eq!(block_on(clip.read_text()).unwrap().as_deref(), Some("hello"));

    let image = ClipboardImage { width: 2, height: 1, rgba: vec![255, 0, 0, 255, 0, 0, 255, 255] };
    block_on(clip.write_image(image.clone())).unwrap();
    assert_eq!(block_on(clip.read_image()).unwrap(), Some(image));
    assert_eq!(block_on(clip.read_text()).unwrap(), None, "an image replaces the text");

    block_on(clip.write_files(vec!["/tmp/a".into()])).unwrap();
    assert_eq!(block_on(clip.read_files()).unwrap(), Some(vec![PathBuf::from("/tmp/a")]));
    assert_eq!(block_on(clip.read_image()).unwrap(), None);

    let bad = ClipboardImage { width: 2, height: 2, rgba: vec![0; 4] };
    assert!(matches!(block_on(clip.write_image(bad)), Err(Error::Invalid(_))));
}

#[test]
fn notification_clicks_echo_their_data() {
    let (sys, fake) = System::fake();
    let mut events = sys.events();
    let n = Notification::new("New message").body("hi").action("reply", "Reply").data(serde_json::json!({ "chat": 7 }));
    let id = block_on(sys.notifications().show(n)).unwrap();
    let plain = block_on(sys.notifications().show(Notification::new("No data"))).unwrap();
    assert_ne!(id, plain);
    match &fake.calls()[0] {
        Call::Notify(shown, n) => {
            assert_eq!(*shown, id);
            assert_eq!(n.data, None, "data stays in this process, the backend never sees it");
            assert_eq!(n.actions[0].id, "reply");
        }
        other => panic!("{other:?}"),
    }
    fake.emit(SystemEvent::NotificationClicked { id: id.clone(), action: Some("reply".into()), data: None });
    fake.emit(SystemEvent::NotificationClicked { id: plain.clone(), action: None, data: None });
    assert_eq!(
        events.try_next(),
        Some(SystemEvent::NotificationClicked {
            id,
            action: Some("reply".into()),
            data: Some(serde_json::json!({ "chat": 7 }))
        })
    );
    assert_eq!(events.try_next(), Some(SystemEvent::NotificationClicked { id: plain, action: None, data: None }));
}

#[test]
fn notification_data_is_capped() {
    let (sys, fake) = System::fake();
    let mut events = sys.events();
    let ids: Vec<_> = (0..130)
        .map(|i| block_on(sys.notifications().show(Notification::new("n").data(serde_json::json!(i)))).unwrap())
        .collect();
    fake.emit(SystemEvent::NotificationClicked { id: ids[0].clone(), action: None, data: None });
    fake.emit(SystemEvent::NotificationClicked { id: ids[129].clone(), action: None, data: None });
    assert!(matches!(events.try_next(), Some(SystemEvent::NotificationClicked { data: None, .. })), "the oldest went");
    assert!(matches!(events.try_next(), Some(SystemEvent::NotificationClicked { data: Some(d), .. }) if d == 129));
}

#[test]
fn recents_and_credentials() {
    let (sys, fake) = System::fake();
    block_on(sys.recent_documents().add("/tmp/a")).unwrap();
    block_on(sys.recent_documents().add("/tmp/b")).unwrap();
    block_on(sys.recent_documents().add("/tmp/a")).unwrap();
    assert_eq!(fake.state().recent, vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")]);
    block_on(sys.recent_documents().clear()).unwrap();
    assert!(fake.state().recent.is_empty());

    let creds = sys.credentials();
    assert_eq!(block_on(creds.get("svc", "me")).unwrap(), None);
    block_on(creds.set("svc", "me", "s3cret")).unwrap();
    assert_eq!(block_on(creds.get("svc", "me")).unwrap().as_deref(), Some("s3cret"));
    block_on(creds.delete("svc", "me")).unwrap();
    block_on(creds.delete("svc", "me")).unwrap();
    assert_eq!(block_on(creds.get("svc", "me")).unwrap(), None);
}

#[test]
fn shell_hands_over_existing_paths_only() {
    let (sys, fake) = System::fake();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("report.txt");
    std::fs::write(&file, "x").unwrap();
    sys.shell().open_url("https://example.org").unwrap();
    sys.shell().reveal_path(&file).unwrap();
    sys.shell().open_path(&file).unwrap();
    assert!(matches!(sys.shell().reveal_path(dir.path().join("missing")), Err(Error::Invalid(_))));
    let canonical = std::fs::canonicalize(&file).unwrap();
    assert_eq!(
        fake.calls(),
        [Call::OpenUrl("https://example.org".into()), Call::RevealPath(canonical.clone()), Call::OpenPath(canonical)]
    );
}

#[test]
fn trays_update_in_place_and_report_choices() {
    let (sys, fake) = System::fake();
    let mut events = sys.events();
    let tray = Tray::new("main")
        .icon(TrayIcon::Named("music.note".into()))
        .tooltip("Player")
        .item(TrayMenuItem::item("play", "Play"))
        .item(TrayMenuItem::Separator)
        .item(TrayMenuItem::check("shuffle", "Shuffle", true).disabled());
    block_on(sys.tray().set(tray.clone())).unwrap();
    block_on(sys.tray().set(tray.clone().title("Now"))).unwrap();
    assert_eq!(fake.state().trays.len(), 1);
    assert_eq!(fake.state().trays["main"].title.as_deref(), Some("Now"));
    assert!(matches!(
        &fake.state().trays["main"].menu[2],
        TrayMenuItem::Item { enabled: false, checked: Some(true), .. }
    ));

    fake.emit(SystemEvent::Tray { tray: TrayId("main".into()), item: Some("play".into()) });
    assert_eq!(events.try_next(), Some(SystemEvent::Tray { tray: TrayId("main".into()), item: Some("play".into()) }));

    block_on(sys.tray().remove(TrayId("main".into()))).unwrap();
    assert!(matches!(block_on(sys.tray().remove(TrayId("main".into()))), Err(Error::NotFound)));
}

#[test]
fn media_session_and_commands() {
    let (sys, fake) = System::fake();
    let mut events = sys.events();
    let now = NowPlaying {
        title: "Song".into(),
        artist: Some("Band".into()),
        duration: Some(Duration::from_secs(200)),
        position: Some(Duration::from_secs(12)),
        state: PlaybackState::Playing,
        can_next: true,
        ..NowPlaying::default()
    };
    block_on(sys.media().set(now.clone())).unwrap();
    assert_eq!(fake.state().now_playing, Some(now));
    fake.emit(SystemEvent::Media(MediaCommand::Next));
    assert_eq!(events.try_next(), Some(SystemEvent::Media(MediaCommand::Next)));
    block_on(sys.media().clear()).unwrap();
    assert_eq!(fake.state().now_playing, None);
}

#[test]
fn audio_players_report_transitions() {
    let (sys, fake) = System::fake();
    let mut events = sys.events();
    let player = block_on(sys.audio().play(PlayOptions::path("/tmp/a.wav").volume(2.).spectrum())).unwrap();
    assert_eq!(fake.state().players[&player.id().0].volume, 1., "volume clamps to 0..=1");
    let state = |events: &mut nd_system::Events| match events.try_next() {
        Some(SystemEvent::Audio { event: AudioEvent::State { state, .. }, .. }) => Some(state),
        _ => None,
    };
    assert_eq!(state(&mut events), Some(AudioState::Playing));
    block_on(player.pause()).unwrap();
    block_on(player.pause()).unwrap();
    block_on(player.seek(Duration::from_secs(3))).unwrap();
    block_on(player.resume()).unwrap();
    assert_eq!(state(&mut events), Some(AudioState::Paused));
    assert_eq!(state(&mut events), Some(AudioState::Playing), "a repeated pause is no transition");
    fake.finish(player.id());
    assert_eq!(state(&mut events), Some(AudioState::Ended));
    block_on(player.stop()).unwrap();
    assert_eq!(state(&mut events), Some(AudioState::Stopped));
    assert!(matches!(block_on(player.resume()), Err(Error::NotFound)), "stop releases the player");
}

#[test]
fn unsupported_features_say_which() {
    let (sys, fake) = System::fake();
    fake.set_unsupported(Feature::Credentials);
    fake.set_unsupported(Feature::Shell);
    match block_on(sys.credentials().get("a", "b")) {
        Err(Error::Unsupported(u)) => assert_eq!(u.feature, Feature::Credentials),
        other => panic!("{other:?}"),
    }
    assert!(matches!(sys.shell().open_url("https://x.org"), Err(Error::Unsupported(_))));
}

#[test]
fn launch_opens_wait_for_the_first_subscriber() {
    let (sys, fake) = System::fake();
    fake.emit(SystemEvent::OpenUrls(vec!["nd-demo://hello".into()]));
    fake.emit(SystemEvent::Activated);
    fake.emit(SystemEvent::FileDrop { window: WindowId(1), paths: vec!["/a".into()], position: None });
    let mut first = sys.events();
    assert_eq!(first.try_next(), Some(SystemEvent::OpenUrls(vec!["nd-demo://hello".into()])));
    assert_eq!(first.try_next(), None, "only open requests are held");
    let mut second = sys.events();
    assert_eq!(second.try_next(), None, "held requests go to one subscriber");
    drop(first);
    fake.emit(SystemEvent::FileDrop { window: WindowId(2), paths: vec!["/b".into()], position: Some((4., 5.)) });
    assert!(matches!(second.try_next(), Some(SystemEvent::FileDrop { window: WindowId(2), .. })));
}

#[test]
fn activation_is_tracked() {
    let (sys, fake) = System::fake();
    assert!(sys.is_active());
    fake.emit(SystemEvent::Deactivated);
    assert!(!sys.is_active());
    sys.emit(SystemEvent::Activated);
    assert!(sys.is_active());
}

#[test]
fn command_lines_become_open_requests() {
    let (sys, _fake) = System::fake();
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("a.md");
    std::fs::write(&doc, "").unwrap();
    let mut events = sys.events();
    sys.open_from_args(["--verbose", "nd-demo://open?x=1", doc.to_str().unwrap(), "C:", "missing.txt", "mailto:a@b.c"]);
    assert_eq!(
        events.try_next(),
        Some(SystemEvent::OpenUrls(vec!["nd-demo://open?x=1".into(), "mailto:a@b.c".into()]))
    );
    assert_eq!(events.try_next(), Some(SystemEvent::OpenFiles(vec![std::fs::canonicalize(&doc).unwrap()])));
    assert_eq!(events.try_next(), None);
}

#[test]
fn events_stream_wakes_an_awaiting_task() {
    let (sys, fake) = System::fake();
    let mut events = sys.events();
    let waiter = std::thread::spawn(move || block_on(events.next()));
    std::thread::sleep(Duration::from_millis(20));
    fake.emit(SystemEvent::Media(MediaCommand::Toggle));
    assert_eq!(waiter.join().unwrap(), Some(SystemEvent::Media(MediaCommand::Toggle)));
}

#[test]
fn directories_follow_the_isolated_root() {
    let root = tempfile::tempdir().unwrap();
    // SAFETY: the only test in this binary that touches the environment; nextest runs each test
    // in its own process.
    unsafe { std::env::set_var("ND_APP_DATA_DIR", root.path()) };
    use nd_system::dirs;
    assert_eq!(dirs::data_dir("notes"), root.path());
    assert_eq!(dirs::cache_dir("notes"), root.path().join("cache"));
    assert_eq!(dirs::config_dir("notes"), root.path().join("config"));
    assert_eq!(dirs::logs_dir("notes"), root.path().join("logs"));
    assert!(dirs::ensure_cache_dir("notes").unwrap().is_dir());
    unsafe { std::env::remove_var("ND_APP_DATA_DIR") };
    let cache = dirs::cache_dir("notes");
    assert!(cache.ends_with("notes") || cache.ends_with("notes/cache"), "{}", cache.display());
}
