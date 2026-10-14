//! Tier 2 on macOS: the Media page's player bar drives real playback through `nd::system`. Play
//! starts the generated tone and the OS player reports it playing; a seek on the bar moves the
//! OS player, which reports the new position when it pauses. Plays a quiet tone for about two
//! seconds and holds the ndshell machine lock. Prints `ND_GALLERY_AUDIO_OK`.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let name = "gate_m6::gallery_audio";
    if args.iter().any(|a| a == "--list") {
        if !args.iter().any(|a| a == "--ignored") && cfg!(target_os = "macos") {
            println!("{name}: test");
        }
        return;
    }
    let filters: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with('-')).collect();
    if args.iter().any(|a| a == "--ignored")
        || !(filters.is_empty() || filters.iter().any(|f| name.contains(f.as_str())))
    {
        return;
    }
    #[cfg(target_os = "macos")]
    audio::run();
}

#[cfg(target_os = "macos")]
mod audio {
    use std::time::{Duration, Instant};

    use nd::test::SemanticAction;

    fn turn(host: &nd::appkit::AppHost) {
        host.platform().pump(Duration::from_millis(20), Duration::from_millis(100));
        host.update(|app| {
            nd::system::run_pending(app);
            app.drain_events();
            app.frame();
        });
    }

    fn until(host: &nd::appkit::AppHost, limit: Duration, done: impl Fn(&nd::appkit::AppHost) -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < limit {
            turn(host);
            if done(host) {
                return true;
            }
        }
        false
    }

    fn act(host: &nd::appkit::AppHost, window: nd::core::WindowId, id: &str, action: SemanticAction) {
        host.update(|app| {
            let tree = app.semantic_window(window).expect("window");
            let node = tree.root.iter().find(|n| n.test_id.as_deref() == Some(id)).expect(id).id;
            app.perform(node, &action).unwrap_or_else(|e| panic!("{id}: {e}"));
        });
    }

    pub fn run() -> ! {
        let data = std::env::temp_dir().join(format!("nd-gallery-audio-{}", std::process::id()));
        unsafe { std::env::set_var("ND_APP_DATA_DIR", &data) };
        let _lock = ndshell::lock::acquire("gallery audio").expect("machine lock");
        let host = gallery::app().host(|_cx| {});
        let start = gallery::Start { page: gallery::Page::Media, audio: true, ..gallery::Start::default() };
        let handle = host.update(|cx| gallery::open(cx, start));
        let window = handle.id();
        let reports = |host: &nd::appkit::AppHost| {
            host.update(|app| handle.root().read(app).media().read(app).reports().to_vec())
        };
        turn(&host);
        let mut failed = 0;
        let mut check = |ok: bool, what: &str| {
            println!("{} {what}", if ok { "ok  " } else { "FAIL" });
            failed += u32::from(!ok);
        };

        act(&host, window, "player.play", SemanticAction::Click);
        let playing = until(&host, Duration::from_secs(5), |h| reports(h).iter().any(|c| c.playing));
        check(playing, "play starts the tone and the OS player reports it playing");

        act(&host, window, "player.seek", SemanticAction::SetValue("0:30".into()));
        until(&host, Duration::from_millis(600), |_| false);
        act(&host, window, "player.play", SemanticAction::Click);
        let paused = until(&host, Duration::from_secs(5), |h| {
            reports(h).last().is_some_and(|c| !c.playing && (29..=33).contains(&c.position.as_secs()))
        });
        let last = reports(&host).last().copied();
        check(paused, &format!("a seek on the bar moves the OS player ({last:?})"));
        let value = host.update(|app| {
            let tree = app.semantic_window(window).expect("window");
            tree.root.iter().find(|n| n.test_id.as_deref() == Some("player.seek")).and_then(|n| n.value.clone())
        });
        check(
            value.as_deref().is_some_and(|v| v.ends_with("of 1:20")),
            &format!("the bar shows the tone's length ({value:?})"),
        );

        host.update(|cx| cx.close_window(window));
        let _ = std::fs::remove_dir_all(&data);
        if failed == 0 {
            println!("ND_GALLERY_AUDIO_OK");
            std::process::exit(0);
        }
        std::process::exit(1);
    }
}
