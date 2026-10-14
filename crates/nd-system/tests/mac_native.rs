//! Tier 2 on macOS: the native backend against AppKit, MediaPlayer, the pasteboard and the
//! login keychain. AppKit needs the main thread, so libtest's harness is off. Dialogs are
//! answered by code (a panel cancelled or confirmed, an alert stopped), never left open.
//! The test touches the user's clipboard and restores its text afterwards.

mod harness;

fn main() {
    let test = harness::Test { name: "gate_m7::mac_native", here: cfg!(target_os = "macos"), ignored: false };
    #[cfg(target_os = "macos")]
    harness::main(test, mac::run);
    #[cfg(not(target_os = "macos"))]
    harness::main(test, || {});
}

#[cfg(target_os = "macos")]
mod mac {
    use std::future::Future;
    use std::path::PathBuf;
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};
    use std::time::{Duration, Instant};

    use nd_system::{
        ClipboardImage, Events, MessageOptions, Notification, NowPlaying, OpenOptions, PlaybackState, SaveOptions,
        System, SystemEvent, Tray, TrayIcon, TrayId, TrayMenuItem, probe,
    };
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{ClassType, msg_send};
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSDocumentController, NSEventMask, NSSavePanel};
    use objc2_foundation::{MainThreadMarker, NSAppleEventDescriptor, NSDate, NSDefaultRunLoopMode, NSString, NSURL};

    fn mtm() -> MainThreadMarker {
        MainThreadMarker::new().expect("main thread")
    }

    /// One turn of AppKit's event loop, Apple events included.
    fn pump() {
        let app = NSApplication::sharedApplication(mtm());
        let until = NSDate::dateWithTimeIntervalSinceNow(0.01);
        // SAFETY: the default mode constant.
        let mode = unsafe { NSDefaultRunLoopMode };
        if let Some(event) =
            app.nextEventMatchingMask_untilDate_inMode_dequeue(NSEventMask::Any, Some(&until), mode, true)
        {
            app.sendEvent(&event);
        }
    }

    /// Polls `fut` while AppKit runs, since the backend answers on the main thread.
    fn wait<T>(fut: impl Future<Output = T>) -> T {
        let mut fut = pin!(fut);
        let mut cx = Context::from_waker(Waker::noop());
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                return v;
            }
            assert!(Instant::now() < deadline, "timed out waiting for the backend");
            pump();
        }
    }

    fn next_event(events: &mut Events, what: &str, matches: impl Fn(&SystemEvent) -> bool) -> SystemEvent {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            while let Some(e) = events.try_next() {
                if matches(&e) {
                    return e;
                }
            }
            assert!(Instant::now() < deadline, "no {what} event");
            pump();
        }
    }

    /// Runs `f` on the main queue after `delay`, while the test pumps or sits in a modal loop.
    fn later(delay: Duration, f: impl FnOnce() + Send + 'static) {
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            dispatch2::DispatchQueue::main().exec_async(f);
        });
    }

    /// The panel on screen; a dismissed one can linger in the window list.
    fn panel() -> Option<Retained<NSSavePanel>> {
        let windows = NSApplication::sharedApplication(mtm()).windows();
        windows.iter().filter(|w| w.isVisible()).filter_map(|w| w.downcast::<NSSavePanel>().ok()).last()
    }

    fn check(name: &str, f: impl FnOnce()) {
        let started = Instant::now();
        f();
        println!("ok {name} ({} ms)", started.elapsed().as_millis());
    }

    pub fn run() {
        // Its panels and alerts take key from any other window test on the machine.
        let _lock = ndshell::lock::window_test("gate_m7::mac_native");
        let app = NSApplication::sharedApplication(mtm());
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        app.finishLaunching();
        let sys = System::native();
        let mut events = sys.events();
        let dir = tempfile::tempdir().unwrap();

        check("clipboard", || {
            let clip = sys.clipboard();
            let before = wait(clip.read_text()).unwrap();
            wait(clip.write_text("nd-system clipboard check")).unwrap();
            assert_eq!(wait(clip.read_text()).unwrap().as_deref(), Some("nd-system clipboard check"));
            let image = ClipboardImage {
                width: 2,
                height: 2,
                rgba: [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 255, 255]].concat(),
            };
            wait(clip.write_image(image.clone())).unwrap();
            let read = wait(clip.read_image()).unwrap().expect("an image");
            assert_eq!((read.width, read.height), (2, 2));
            let file = dir.path().join("copied.txt");
            std::fs::write(&file, "x").unwrap();
            wait(clip.write_files(vec![file.clone()])).unwrap();
            let files = wait(clip.read_files()).unwrap().expect("files");
            assert_eq!(std::fs::canonicalize(&files[0]).unwrap(), std::fs::canonicalize(&file).unwrap());
            if let Some(before) = before {
                wait(clip.write_text(before)).unwrap();
            }
        });

        check("credentials in the login keychain", || {
            let service = format!("dev.nd.system-test.{}", std::process::id());
            let creds = sys.credentials();
            assert_eq!(wait(creds.get(&service, "me")).unwrap(), None);
            wait(creds.set(&service, "me", "s3cret")).unwrap();
            assert_eq!(wait(creds.get(&service, "me")).unwrap().as_deref(), Some("s3cret"));
            wait(creds.set(&service, "me", "rotated")).unwrap();
            assert_eq!(wait(creds.get(&service, "me")).unwrap().as_deref(), Some("rotated"));
            wait(creds.delete(&service, "me")).unwrap();
            assert_eq!(wait(creds.get(&service, "me")).unwrap(), None);
        });

        check("open panel, cancelled", || {
            let pending = sys.dialogs().open(OpenOptions::file().title("nd-system test").starting_in(dir.path()));
            later(Duration::from_millis(300), || {
                // SAFETY: a nil sender, as the Cancel button may pass.
                unsafe { panel().expect("open panel is up").cancel(None) };
            });
            assert_eq!(wait(pending).unwrap(), None);
        });

        check("save panel, configured then cancelled", || {
            // The save panel runs out of process: `ok:` is not implemented on it, so the test
            // reads back what the options set and cancels.
            let pending = sys.dialogs().save(SaveOptions::new().starting_in(dir.path()).file_name("saved.txt"));
            let expected_dir = std::fs::canonicalize(dir.path()).unwrap();
            later(Duration::from_millis(300), move || {
                let panel = panel().expect("save panel is up");
                assert_eq!(panel.nameFieldStringValue().to_string(), "saved.txt");
                let shown = panel.directoryURL().and_then(|u| u.path()).map(|p| PathBuf::from(p.to_string())).unwrap();
                assert_eq!(std::fs::canonicalize(shown).unwrap(), expected_dir);
                // SAFETY: a nil sender, as the Cancel button may pass.
                unsafe { panel.cancel(None) };
            });
            assert_eq!(wait(pending).unwrap(), None);
        });

        check("app modal alert", || {
            let alert =
                MessageOptions::new("nd-system test").detail("Answered by the test").button("Keep").button("Discard");
            let pending = sys.dialogs().message(alert);
            later(Duration::from_millis(300), || {
                // NSAlertSecondButtonReturn.
                NSApplication::sharedApplication(MainThreadMarker::new().unwrap()).stopModalWithCode(1001);
            });
            assert_eq!(wait(pending).unwrap(), 1);
        });

        check("status item", || {
            let id = TrayId("nd-test".into());
            let tray = Tray::new("nd-test")
                .icon(TrayIcon::Named("music.note".into()))
                .title("ND")
                .item(TrayMenuItem::item("play", "Play"))
                .item(TrayMenuItem::Separator)
                .item(TrayMenuItem::check("shuffle", "Shuffle", true));
            wait(sys.tray().set(tray.clone())).unwrap();
            assert_eq!(probe::tray(&id), Some(("ND".into(), vec!["Play".into(), String::new(), "Shuffle".into()])));
            assert!(probe::tray_perform(&id, Some(2)));
            let chosen = next_event(&mut events, "tray", |e| matches!(e, SystemEvent::Tray { .. }));
            assert_eq!(chosen, SystemEvent::Tray { tray: id.clone(), item: Some("shuffle".into()) });

            let bare = Tray { menu: Vec::new(), ..tray.title("Bare") };
            wait(sys.tray().set(bare)).unwrap();
            assert_eq!(probe::tray(&id).unwrap().1, Vec::<String>::new(), "updated in place, menu gone");
            assert!(probe::tray_perform(&id, None));
            let clicked = next_event(&mut events, "tray click", |e| matches!(e, SystemEvent::Tray { .. }));
            assert_eq!(clicked, SystemEvent::Tray { tray: id.clone(), item: None });

            wait(sys.tray().remove(id.clone())).unwrap();
            assert_eq!(probe::tray(&id), None);
            assert!(wait(sys.tray().remove(id)).is_err());
            let bad = Tray::new("bad").icon(TrayIcon::Named("no.such.symbol.nd".into()));
            assert!(wait(sys.tray().set(bad)).is_err());
        });

        check("now playing", || {
            let now = NowPlaying {
                title: "nd-system test".into(),
                artist: Some("Band".into()),
                duration: Some(Duration::from_secs(200)),
                position: Some(Duration::from_secs(5)),
                state: PlaybackState::Paused,
                can_next: true,
                ..NowPlaying::default()
            };
            wait(sys.media().set(now)).unwrap();
            assert_eq!(probe::now_playing(), (Some("nd-system test".into()), 2));
            wait(sys.media().clear()).unwrap();
            assert_eq!(probe::now_playing(), (None, 3));
        });

        check("recent documents", || {
            let file = dir.path().join("recent.txt");
            std::fs::write(&file, "x").unwrap();
            wait(sys.recent_documents().add(&file)).unwrap();
            let controller = NSDocumentController::sharedDocumentController(mtm());
            let listed: Vec<PathBuf> = controller
                .recentDocumentURLs()
                .iter()
                .filter_map(|u| u.path())
                .map(|p| PathBuf::from(p.to_string()))
                .collect();
            assert!(listed.iter().any(|p| p.ends_with("recent.txt")), "{listed:?}");
            wait(sys.recent_documents().clear()).unwrap();
            assert_eq!(controller.recentDocumentURLs().count(), 0);
        });

        check("notification without a bundle", || {
            // Without a bundle identifier UserNotifications is off limits; the legacy center
            // either delivers or the call says why it cannot, and nothing raises.
            match wait(sys.notifications().show(Notification::new("nd-system test").body("legacy path").silent())) {
                Ok(id) => assert!(id.0.starts_with("nd-")),
                Err(e) => assert!(e.to_string().contains("bundled app"), "{e}"),
            }
        });

        check("open URL and documents Apple events", || {
            send_get_url("nd-system-test://hello?x=1");
            let opened = next_event(&mut events, "open URL", |e| matches!(e, SystemEvent::OpenUrls(_)));
            assert_eq!(opened, SystemEvent::OpenUrls(vec!["nd-system-test://hello?x=1".into()]));
            let doc = dir.path().join("doc.md");
            std::fs::write(&doc, "").unwrap();
            send_open_documents(&doc);
            let opened = next_event(&mut events, "open documents", |e| matches!(e, SystemEvent::OpenFiles(_)));
            let SystemEvent::OpenFiles(paths) = opened else { unreachable!() };
            assert_eq!(std::fs::canonicalize(&paths[0]).unwrap(), std::fs::canonicalize(&doc).unwrap());
        });

        check("shell refuses a missing path", || {
            assert!(sys.shell().reveal_path(dir.path().join("missing")).is_err());
        });

        println!("ND_SYSTEM_MAC_OK");
    }

    const fn four(code: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*code)
    }

    fn apple_event(class: &[u8; 4], id: &[u8; 4], direct: &NSAppleEventDescriptor) {
        let target = NSAppleEventDescriptor::currentProcessDescriptor();
        // SAFETY: Apple event class, id and keyword are FourCharCodes; kAutoGenerateReturnID is
        // -1 and kAnyTransactionID 0; kAENoReply is 1, so sending to ourselves cannot block.
        unsafe {
            let event: Retained<NSAppleEventDescriptor> = msg_send![
                NSAppleEventDescriptor::class(),
                appleEventWithEventClass: four(class), eventID: four(id), targetDescriptor: &*target, returnID: -1i16, transactionID: 0i32
            ];
            let _: () = msg_send![&*event, setParamDescriptor: direct, forKeyword: four(b"----")];
            let error: *mut AnyObject = std::ptr::null_mut();
            let _: Option<Retained<NSAppleEventDescriptor>> =
                msg_send![&*event, sendEventWithOptions: 1u64, timeout: 5.0f64, error: &error];
        }
    }

    fn send_get_url(url: &str) {
        apple_event(b"GURL", b"GURL", &NSAppleEventDescriptor::descriptorWithString(&NSString::from_str(url)));
    }

    fn send_open_documents(path: &std::path::Path) {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        let list = NSAppleEventDescriptor::listDescriptor();
        // SAFETY: `descriptorWithFileURL:` (10.11) and `insertDescriptor:atIndex:` with index 0,
        // which appends.
        unsafe {
            let file: Retained<NSAppleEventDescriptor> =
                msg_send![NSAppleEventDescriptor::class(), descriptorWithFileURL: &*url];
            let _: () = msg_send![&*list, insertDescriptor: &*file, atIndex: 0isize];
        }
        apple_event(b"aevt", b"odoc", &list);
    }
}
