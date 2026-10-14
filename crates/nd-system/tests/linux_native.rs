//! Tier 2 on Linux: the native backend over a private session bus (`dbus-run-session`). The
//! test serves a fake notification daemon on that bus, drives the tray and MPRIS objects the
//! backend exports the way a panel and `playerctl` would, round-trips a secret through the
//! Secret Service when one runs there, edits an isolated `recently-used.xbel`, and copies
//! through a headless Hyprland's clipboard. Run it as
//! `dbus-run-session -- cargo test -p nd-system --test linux_native`.

#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
fn main() {
    linux::run();
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use futures_executor::block_on;
    use nd_system::{
        AppIdentity, ClipboardImage, Events, MediaCommand, Notification, NowPlaying, OpenOptions, PlaybackState,
        System, SystemEvent, Tray, TrayIcon, TrayId, TrayMenuItem,
    };
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedValue, Structure};

    const APP_ID: &str = "dev.nd.systemtest";

    fn next_event(events: &mut Events, what: &str, matches: impl Fn(&SystemEvent) -> bool) -> SystemEvent {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            while let Some(e) = events.try_next() {
                if matches(&e) {
                    return e;
                }
            }
            assert!(Instant::now() < deadline, "no {what} event");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn check(name: &str, f: impl FnOnce() -> Result<(), String>) {
        let started = Instant::now();
        match f() {
            Ok(()) => println!("ok {name} ({} ms)", started.elapsed().as_millis()),
            Err(skip) => println!("skip {name}: {skip}"),
        }
    }

    #[derive(Default)]
    struct Shown {
        summaries: Vec<String>,
        bodies: Vec<String>,
        actions: Vec<Vec<String>>,
    }

    struct FakeDaemon {
        shown: Arc<Mutex<Shown>>,
    }

    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl FakeDaemon {
        #[allow(clippy::too_many_arguments)]
        fn notify(
            &self,
            _app_name: String,
            _replaces_id: u32,
            _app_icon: String,
            summary: String,
            body: String,
            actions: Vec<String>,
            _hints: HashMap<String, OwnedValue>,
            _expire_timeout: i32,
        ) -> u32 {
            let mut shown = self.shown.lock().unwrap();
            shown.summaries.push(summary);
            shown.bodies.push(body);
            shown.actions.push(actions);
            shown.summaries.len() as u32
        }

        fn get_capabilities(&self) -> Vec<String> {
            vec!["actions".into(), "body".into()]
        }

        fn get_server_information(&self) -> (String, String, String, String) {
            ("nd-fake".into(), "nd".into(), "1".into(), "1.2".into())
        }

        fn close_notification(&self, _id: u32) {}
    }

    /// The layout's top-level items as (id, label).
    fn menu_items(bus: &Connection, name: &str) -> Vec<(i32, String)> {
        let menu = Proxy::new(bus, name.to_owned(), "/MenuBar", "com.canonical.dbusmenu").unwrap();
        type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);
        let (_revision, root): (u32, Layout) = menu.call("GetLayout", &(0i32, -1i32, Vec::<String>::new())).unwrap();
        root.2
            .into_iter()
            .map(|child| {
                let fields = Structure::try_from(child).unwrap().into_fields();
                let id = i32::try_from(&fields[0]).unwrap();
                let props = HashMap::<String, OwnedValue>::try_from(fields[1].try_clone().unwrap()).unwrap();
                let label =
                    props.get("label").and_then(|v| String::try_from(v.try_clone().unwrap()).ok()).unwrap_or_default();
                (id, label)
            })
            .collect()
    }

    pub fn run() {
        let bus = Connection::session().expect("a session bus: run under dbus-run-session");
        let data = tempfile::tempdir().unwrap();
        // SAFETY: set before any thread reads them; this binary runs one test.
        unsafe { std::env::set_var("XDG_DATA_HOME", data.path()) };
        nd_system::set_app_identity(AppIdentity {
            id: APP_ID.into(),
            name: "systemtest".into(),
            display_name: "System Test".into(),
        });
        let sys = System::native();
        let mut events = sys.events();

        check("notifications through the session daemon", || {
            let shown = Arc::new(Mutex::new(Shown::default()));
            let daemon = zbus::blocking::connection::Builder::session()
                .and_then(|b| b.name("org.freedesktop.Notifications"))
                .and_then(|b| b.serve_at("/org/freedesktop/Notifications", FakeDaemon { shown: shown.clone() }))
                .and_then(|b| b.build())
                .map_err(|e| e.to_string())?;
            let n = Notification::new("Hello")
                .subtitle("from nd")
                .body("body text")
                .action("reply", "Reply")
                .data(serde_json::json!(42));
            let id = block_on(sys.notifications().show(n)).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while shown.lock().unwrap().summaries.is_empty() {
                assert!(Instant::now() < deadline, "the daemon never got Notify");
                std::thread::sleep(Duration::from_millis(10));
            }
            {
                let shown = shown.lock().unwrap();
                assert_eq!(shown.summaries, ["Hello"]);
                assert_eq!(shown.bodies, ["from nd\nbody text"]);
                assert_eq!(shown.actions[0], ["default", "", "reply", "Reply"]);
            }
            // The handle subscribes to ActionInvoked right after Notify returns.
            std::thread::sleep(Duration::from_millis(200));
            daemon
                .emit_signal(
                    None::<()>,
                    "/org/freedesktop/Notifications",
                    "org.freedesktop.Notifications",
                    "ActionInvoked",
                    &(1u32, "reply"),
                )
                .map_err(|e| e.to_string())?;
            let clicked =
                next_event(&mut events, "notification click", |e| matches!(e, SystemEvent::NotificationClicked { .. }));
            assert_eq!(
                clicked,
                SystemEvent::NotificationClicked {
                    id,
                    action: Some("reply".into()),
                    data: Some(serde_json::json!(42))
                }
            );
            Ok(())
        });

        check("status notifier item", || {
            let tray = Tray::new("nd-test")
                .icon(TrayIcon::Named("media-playback-start".into()))
                .tooltip("ND test")
                .item(TrayMenuItem::item("play", "Play"))
                .item(TrayMenuItem::Separator)
                .item(TrayMenuItem::check("shuffle", "Shuffle", true));
            block_on(sys.tray().set(tray)).map_err(|e| e.to_string())?;
            let dbus = zbus::blocking::fdo::DBusProxy::new(&bus).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            let name = loop {
                let names = dbus.list_names().unwrap();
                if let Some(n) = names.iter().find(|n| n.as_str().starts_with("org.kde.StatusNotifierItem-")) {
                    break n.to_string();
                }
                assert!(Instant::now() < deadline, "no StatusNotifierItem name on the bus");
                std::thread::sleep(Duration::from_millis(20));
            };
            let item = Proxy::new(&bus, name.clone(), "/StatusNotifierItem", "org.kde.StatusNotifierItem").unwrap();
            assert_eq!(item.get_property::<String>("Id").unwrap(), "nd-test");
            assert_eq!(item.get_property::<String>("IconName").unwrap(), "media-playback-start");
            let _: () = item.call("Activate", &(0i32, 0i32)).unwrap();
            let clicked = next_event(&mut events, "tray activate", |e| matches!(e, SystemEvent::Tray { .. }));
            assert_eq!(clicked, SystemEvent::Tray { tray: TrayId("nd-test".into()), item: None });

            let items = menu_items(&bus, &name);
            let labels: Vec<&str> = items.iter().map(|(_, l)| l.as_str()).collect();
            assert_eq!(labels, ["Play", "", "Shuffle"]);
            let menu = Proxy::new(&bus, name.clone(), "/MenuBar", "com.canonical.dbusmenu").unwrap();
            let shuffle = items[2].0;
            let _: () = menu.call("Event", &(shuffle, "clicked", zbus::zvariant::Value::from(0i32), 0u32)).unwrap();
            let chosen = next_event(&mut events, "tray menu", |e| matches!(e, SystemEvent::Tray { .. }));
            assert_eq!(chosen, SystemEvent::Tray { tray: TrayId("nd-test".into()), item: Some("shuffle".into()) });

            block_on(sys.tray().remove(TrayId("nd-test".into()))).unwrap();
            Ok(())
        });

        check("MPRIS", || {
            let now = NowPlaying {
                title: "Song".into(),
                artist: Some("Band".into()),
                duration: Some(Duration::from_secs(200)),
                state: PlaybackState::Playing,
                can_next: true,
                ..NowPlaying::default()
            };
            block_on(sys.media().set(now)).map_err(|e| e.to_string())?;
            let player = Proxy::new(
                &bus,
                format!("org.mpris.MediaPlayer2.{APP_ID}"),
                "/org/mpris/MediaPlayer2",
                "org.mpris.MediaPlayer2.Player",
            )
            .unwrap();
            assert_eq!(player.get_property::<String>("PlaybackStatus").unwrap(), "Playing");
            assert!(player.get_property::<bool>("CanGoNext").unwrap());
            let metadata: HashMap<String, OwnedValue> = player.get_property("Metadata").unwrap();
            assert_eq!(String::try_from(metadata["xesam:title"].try_clone().unwrap()).unwrap(), "Song");
            let root = Proxy::new(
                &bus,
                format!("org.mpris.MediaPlayer2.{APP_ID}"),
                "/org/mpris/MediaPlayer2",
                "org.mpris.MediaPlayer2",
            )
            .unwrap();
            assert_eq!(root.get_property::<String>("Identity").unwrap(), "System Test");
            for (method, command) in [("PlayPause", MediaCommand::Toggle), ("Next", MediaCommand::Next)] {
                let _: () = player.call(method, &()).unwrap();
                let got = next_event(&mut events, method, |e| matches!(e, SystemEvent::Media(_)));
                assert_eq!(got, SystemEvent::Media(command));
            }
            let _: () = player.call("Seek", &(-5_000_000i64)).unwrap();
            assert_eq!(
                next_event(&mut events, "Seek", |e| matches!(e, SystemEvent::Media(_))),
                SystemEvent::Media(MediaCommand::SeekBy(-5000))
            );
            block_on(sys.media().clear()).unwrap();
            let dbus = zbus::blocking::fdo::DBusProxy::new(&bus).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while dbus.list_names().unwrap().iter().any(|n| n.as_str() == format!("org.mpris.MediaPlayer2.{APP_ID}")) {
                assert!(Instant::now() < deadline, "the player stayed on the bus after clear");
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(())
        });

        check("credentials through the Secret Service", || {
            // gnome-keyring's `--unlock` makes an unlocked `login` collection; without a
            // `default` alias it would prompt for a new collection, which nobody answers here.
            let secrets = Proxy::new(
                &bus,
                "org.freedesktop.secrets",
                "/org/freedesktop/secrets",
                "org.freedesktop.Secret.Service",
            )
            .map_err(|e| e.to_string())?;
            let login = zbus::zvariant::ObjectPath::try_from("/org/freedesktop/secrets/collection/login").unwrap();
            let _: () = secrets.call("SetAlias", &("default", login)).map_err(|e| format!("no Secret Service: {e}"))?;
            let service = format!("dev.nd.system-test.{}", std::process::id());
            let (tx, rx) = std::sync::mpsc::channel();
            let creds_sys = sys.clone();
            std::thread::spawn(move || {
                let creds = creds_sys.credentials();
                let run = || -> nd_system::Result<()> {
                    assert_eq!(block_on(creds.get(&service, "me"))?, None);
                    block_on(creds.set(&service, "me", "s3cret"))?;
                    assert_eq!(block_on(creds.get(&service, "me"))?.as_deref(), Some("s3cret"));
                    block_on(creds.delete(&service, "me"))?;
                    assert_eq!(block_on(creds.get(&service, "me"))?, None);
                    Ok(())
                };
                let _ = tx.send(run());
            });
            match rx.recv_timeout(Duration::from_secs(20)) {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => panic!("{e}"),
                Err(_) => panic!("the Secret Service did not answer in 20 s (a prompt?)"),
            }
        });

        check("recently-used.xbel", || {
            let doc = data.path().join("doc one.md");
            std::fs::write(&doc, "").unwrap();
            block_on(sys.recent_documents().add(&doc)).unwrap();
            let xbel = std::fs::read_to_string(data.path().join("recently-used.xbel")).unwrap();
            assert!(xbel.contains("doc%20one.md") && xbel.contains("name=\"System Test\""), "{xbel}");
            block_on(sys.recent_documents().clear()).unwrap();
            let xbel = std::fs::read_to_string(data.path().join("recently-used.xbel")).unwrap();
            assert!(!xbel.contains("<bookmark "), "{xbel}");
            Ok(())
        });

        check("scripted dialogs", || {
            let script = nd_system::DialogScript::parse(r#"{ "dialog.openFile": [{ "paths": ["/tmp/x"] }] }"#).unwrap();
            let (fake, _) = System::fake();
            let scripted = fake.with_dialog_script(Some(script));
            assert!(block_on(scripted.dialogs().open(OpenOptions::file())).unwrap().is_some());
            Ok(())
        });

        check("clipboard through a headless Hyprland", || {
            let hyprland = ndshell::hyprland::Hyprland::start(Default::default())?;
            // SAFETY: as above; the clipboard thread connects on its first job, after this.
            unsafe {
                std::env::remove_var("DISPLAY");
                for (key, value) in hyprland.client_env() {
                    std::env::set_var(key, value);
                }
            }
            let clip = sys.clipboard();
            block_on(clip.write_text("nd linux clipboard")).map_err(|e| e.to_string())?;
            assert_eq!(block_on(clip.read_text()).unwrap().as_deref(), Some("nd linux clipboard"));
            let image = ClipboardImage { width: 1, height: 2, rgba: vec![255, 0, 0, 255, 0, 0, 255, 255] };
            block_on(clip.write_image(image.clone())).unwrap();
            assert_eq!(block_on(clip.read_image()).unwrap(), Some(image));
            let file = data.path().join("copied.txt");
            std::fs::write(&file, "x").unwrap();
            block_on(clip.write_files(vec![file.clone()])).unwrap();
            assert_eq!(block_on(clip.read_files()).unwrap(), Some(vec![file]));
            Ok(())
        });

        println!("ND_SYSTEM_LINUX_OK");
    }
}
