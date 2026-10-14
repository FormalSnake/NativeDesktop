//! M10 gate: `nd new` + `nd package mac` gives a launching `.app` (`nd package windows` a
//! branded Setup.exe that installs and launches it); 0.1.0 updates to 0.1.1 through
//! `nd release serve` with a signed delta, applied on quit; a tampered feed is refused; every
//! publisher writes to a local target.

mod gate_m10 {
    #[cfg(any(target_os = "macos", windows))]
    use std::path::{Path, PathBuf};
    #[cfg(any(target_os = "macos", windows))]
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex};
    #[cfg(any(target_os = "macos", windows))]
    use std::time::{Duration, Instant};

    #[cfg(unix)]
    use nd_cli::publish::GhCommand;
    use nd_cli::publish::{self, S3Target};
    use nd_config::Publish;

    fn feed_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, body) in [
            ("releases.mac-stable.json", "{}"),
            ("releases.mac-stable.json.sig", "sig"),
            ("app-0.1.1-mac-stable-full.nupkg", "full"),
            ("app-0.1.1-mac-stable-delta.nupkg", "delta"),
        ] {
            std::fs::write(dir.path().join(name), body).unwrap();
        }
        dir
    }

    const PUBLISHED: [&str; 4] = [
        "app-0.1.1-mac-stable-delta.nupkg",
        "app-0.1.1-mac-stable-full.nupkg",
        "releases.mac-stable.json",
        "releases.mac-stable.json.sig",
    ];

    #[test]
    fn publish_dir() {
        let src = feed_dir();
        let root = tempfile::tempdir().unwrap();
        publish::publish(&Publish::Dir { path: "feed".into() }, src.path(), root.path()).unwrap();
        assert_eq!(nd_cli::package::listing(&root.path().join("feed")), PUBLISHED);
    }

    #[test]
    fn publish_rsync_to_a_local_target() {
        if nd_cli::vpk::which("rsync").is_none() {
            eprintln!("skipped: no rsync on PATH");
            return;
        }
        let src = feed_dir();
        let dest = tempfile::tempdir().unwrap();
        let target = format!("{}/", dest.path().display());
        publish::publish(&Publish::Rsync { target }, src.path(), dest.path()).unwrap();
        assert_eq!(nd_cli::package::listing(dest.path()), PUBLISHED);
    }

    /// No MinIO: a local HTTP server stands in for the bucket and records the presigned PUTs.
    #[test]
    fn publish_s3_against_a_local_endpoint() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let handle = std::thread::spawn(move || {
            for _ in 0..PUBLISHED.len() {
                let mut request = server.recv().unwrap();
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                log.lock().unwrap().push((request.method().to_string(), request.url().to_owned(), body));
                request.respond(tiny_http::Response::empty(200)).unwrap();
            }
        });
        let src = feed_dir();
        let target = S3Target {
            endpoint: format!("http://127.0.0.1:{port}"),
            bucket: "updates".into(),
            prefix: Some("demo/".into()),
            region: "auto".into(),
        };
        let files = publish::upload_order(src.path()).unwrap();
        publish::to_s3(&files, &target, &rusty_s3::Credentials::new("key", "secret")).unwrap();
        handle.join().unwrap();
        let seen = seen.lock().unwrap();
        let paths: Vec<String> = seen.iter().map(|(_, url, _)| url.split('?').next().unwrap().to_owned()).collect();
        let want: Vec<String> = PUBLISHED.iter().map(|n| format!("/updates/demo/{n}")).collect();
        assert_eq!(paths, want);
        assert!(seen.iter().all(|(method, url, _)| method == "PUT" && url.contains("X-Amz-Signature=")));
        assert_eq!(seen[3].2, "sig");
    }

    /// No network: `gh` is a script that logs its arguments and has no release yet.
    #[cfg(unix)]
    #[test]
    fn publish_github_through_gh() {
        use std::os::unix::fs::PermissionsExt as _;
        let tools = tempfile::tempdir().unwrap();
        let log = tools.path().join("gh.log");
        let gh = tools.path().join("gh");
        let script = format!("#!/bin/sh\necho \"$@\" >> '{}'\n[ \"$2\" = view ] && exit 1\nexit 0\n", log.display());
        std::fs::write(&gh, script).unwrap();
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        let src = feed_dir();
        let files = publish::upload_order(src.path()).unwrap();
        publish::github(&files, Some("formalsnake/demo"), &GhCommand { program: Some(gh) }).unwrap();
        let calls: Vec<String> = std::fs::read_to_string(log).unwrap().lines().map(str::to_owned).collect();
        assert_eq!(calls.len(), 4, "{calls:?}");
        assert!(calls[0].starts_with("release view updates"));
        assert!(calls[1].starts_with("release create updates"));
        assert!(calls[2].contains("full.nupkg") && !calls[2].contains("releases."));
        assert!(calls[3].contains("releases.mac-stable.json.sig") && calls[3].ends_with("--repo formalsnake/demo"));
    }

    // The full flow below is macOS only for now: `nd package linux` and `windows` are covered by
    // the unit tests of their argument and AppDir builders.

    #[cfg(target_os = "macos")]
    const APP_ID: &str = "dev.formalsnake.ndgatem10";

    #[cfg(any(target_os = "macos", windows))]
    fn nd(dir: &Path, args: &[&str]) -> String {
        let output = Command::new(env!("CARGO_BIN_EXE_nd")).args(args).current_dir(dir).output().expect("spawn nd");
        assert!(
            output.status.success(),
            "nd {args:?} failed\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    #[cfg(target_os = "macos")]
    fn system(program: &str, args: &[&str]) {
        let status =
            Command::new(program).args(args).env_remove("SDKROOT").env_remove("DEVELOPER_DIR").status().unwrap();
        assert!(status.success(), "{program} {args:?} failed ({status})");
    }

    #[cfg(target_os = "macos")]
    fn bundle_version(app: &Path) -> String {
        let out = Command::new("/usr/bin/plutil")
            .args(["-extract", "CFBundleShortVersionString", "raw"])
            .arg(app.join("Contents/Info.plist"))
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    #[cfg(target_os = "macos")]
    fn velopack_packages() -> PathBuf {
        PathBuf::from(std::env::var_os("HOME").unwrap()).join("Library/Caches/velopack").join(APP_ID)
    }

    #[cfg(any(target_os = "macos", windows))]
    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    #[cfg(any(target_os = "macos", windows))]
    struct Serve {
        child: Child,
        log: PathBuf,
    }

    #[cfg(any(target_os = "macos", windows))]
    impl Serve {
        fn start(dir: &Path, root: &Path, port: u16, log: PathBuf) -> Self {
            let out = std::fs::File::create(&log).unwrap();
            let child = Command::new(env!("CARGO_BIN_EXE_nd"))
                .args(["release", "serve", "--port", &port.to_string(), "--root"])
                .arg(root)
                .current_dir(dir)
                .stdout(out)
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let start = Instant::now();
            while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
                assert!(start.elapsed() < Duration::from_secs(10), "nd release serve did not listen");
                std::thread::sleep(Duration::from_millis(50));
            }
            Self { child, log }
        }

        fn requests(&self) -> Vec<String> {
            std::fs::read_to_string(&self.log).unwrap_or_default().lines().skip(1).map(str::to_owned).collect()
        }
    }

    #[cfg(any(target_os = "macos", windows))]
    impl Drop for Serve {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[cfg(target_os = "macos")]
    fn launch(app: &Path) -> nd_test::AppHandle {
        nd_test::launch_app(
            nd_test::LaunchOptions::new(app.join("Contents/MacOS/demo")).env("ND_UPDATE_DELAY_MS", "200"),
        )
        .expect("launch the packaged app")
    }

    #[cfg(any(target_os = "macos", windows))]
    fn wait_until(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < timeout, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn real_package_mac_then_signed_delta_update() {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("nd-m10-gate");
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(velopack_packages());
        std::fs::create_dir_all(&base).unwrap();
        let nd_crate = Path::new(env!("CARGO_MANIFEST_DIR")).join("../nd");

        // nd new demo, nd package mac: the .app launches and opens its window.
        nd(&base, &["new", "demo", "--standalone", "--id", APP_ID, "--nd-path", nd_crate.to_str().unwrap()]);
        let demo = base.join("demo");
        let out = nd(&demo, &["package", "mac"]);
        let packaged = demo.join("target/nd/package/mac/out/Demo.app");
        assert!(out.contains("Demo-0.1.0.dmg"), "{out}");
        system("/usr/bin/codesign", &["--verify", "--deep", "--strict", packaged.to_str().unwrap()]);
        {
            let app = launch(&packaged);
            let windows = app.windows().unwrap();
            assert!(windows.as_array().is_some_and(|w| !w.is_empty()), "no window: {windows}");
        }

        // Signing key and feed.
        let port = free_port();
        let key = base.join("signing.key");
        let keygen = nd(&demo, &["release", "keygen", "--out", key.to_str().unwrap()]);
        let config = std::fs::read_to_string(demo.join("nativedesktop.toml")).unwrap();
        std::fs::write(
            demo.join("nativedesktop.toml"),
            format!("{config}\n[updates]\nfeed = \"http://127.0.0.1:{port}\"\n{keygen}"),
        )
        .unwrap();

        // 0.1.0 released and installed. Velopack keeps the installed version's full package in
        // its cache once an install has updated before; seeding it is what makes this first
        // update a delta (a fresh .pkg install clears that cache).
        nd(&demo, &["release", "--key", key.to_str().unwrap()]);
        let feed = demo.join("target/nd/releases");
        // Velopack stages the new bundle in ~/Library/Caches and renames it over the old one, so
        // the install must sit on that volume (the target dir is on an external drive here).
        let caches = PathBuf::from(std::env::var_os("HOME").unwrap()).join("Library/Caches");
        let installs = tempfile::Builder::new().prefix("nd-m10-gate").tempdir_in(caches).unwrap();
        let install = installs.path().join("install");
        let portable = feed.join(format!("{APP_ID}-mac-stable-Portable.zip"));
        system("/usr/bin/ditto", &["-x", "-k", portable.to_str().unwrap(), install.to_str().unwrap()]);
        system("/usr/bin/ditto", &[install.to_str().unwrap(), installs.path().join("install-0.1.0").to_str().unwrap()]);
        let installed = install.join("Demo.app");
        assert_eq!(bundle_version(&installed), "0.1.0");
        std::fs::create_dir_all(velopack_packages().join("packages")).unwrap();
        let full_010 = format!("{APP_ID}-0.1.0-mac-stable-full.nupkg");
        std::fs::copy(feed.join(&full_010), velopack_packages().join("packages").join(&full_010)).unwrap();

        // 0.1.1 with notes.
        let cargo = std::fs::read_to_string(demo.join("Cargo.toml")).unwrap();
        std::fs::write(demo.join("Cargo.toml"), cargo.replacen("version = \"0.1.0\"", "version = \"0.1.1\"", 1))
            .unwrap();
        let app_rs = std::fs::read_to_string(demo.join("src/app.rs")).unwrap();
        std::fs::write(demo.join("src/app.rs"), app_rs.replace("text(\"clicks\")", "text(\"clicks in 0.1.1\")"))
            .unwrap();
        let notes = base.join("notes.md");
        std::fs::write(&notes, "The counter label names its version.\n").unwrap();
        nd(&demo, &["release", "--key", key.to_str().unwrap(), "--notes", notes.to_str().unwrap()]);
        let delta = format!("{APP_ID}-0.1.1-mac-stable-delta.nupkg");
        let full_011 = format!("{APP_ID}-0.1.1-mac-stable-full.nupkg");
        assert!(feed.join(&delta).exists(), "no delta in {:?}", nd_cli::package::listing(&feed));

        // The installed 0.1.0 finds 0.1.1, downloads the delta, applies it when it quits.
        let serve = Serve::start(&demo, &feed, port, base.join("serve.log"));
        {
            let app = launch(&installed);
            app.wait_for_stderr("update 0.1.1 ready", Duration::from_secs(60)).unwrap();
            let pending = std::fs::read_to_string(app.dir().join("data/updates/pending.json")).unwrap();
            assert!(pending.contains("names its version"), "what's new record: {pending}");
            app.keyboard().press("cmd-q").unwrap();
            wait_until("0.1.1 applied on quit", Duration::from_secs(90), || bundle_version(&installed) == "0.1.1");
        }
        system("/usr/bin/codesign", &["--verify", "--deep", "--strict", installed.to_str().unwrap()]);
        let requests = serve.requests();
        assert!(requests.contains(&"GET /releases.mac-stable.json.sig 200".to_string()), "{requests:?}");
        assert!(requests.contains(&format!("GET /{delta} 200")), "{requests:?}");
        assert!(!requests.iter().any(|r| r.contains(&full_011)), "fell back to the full package: {requests:?}");

        // A tampered feed (same signature, edited bytes) is refused before any download.
        let _ = std::fs::remove_dir_all(velopack_packages());
        let json_path = feed.join("releases.mac-stable.json");
        let json = std::fs::read_to_string(&json_path).unwrap();
        std::fs::write(&json_path, json.replace("names its version", "installs something else")).unwrap();
        let before = serve.requests().len();
        let old = installs.path().join("install-0.1.0/Demo.app");
        {
            let app = launch(&old);
            let line = app.wait_for_stderr("nd: update", Duration::from_secs(60)).unwrap();
            assert!(line.contains("does not match"), "{line}");
        }
        let after = serve.requests();
        assert!(after[before..].iter().all(|r| !r.contains(".nupkg")), "downloaded from a tampered feed: {after:?}");
        assert_eq!(bundle_version(&old), "0.1.0");

        drop(serve);
        let _ = std::fs::remove_dir_all(velopack_packages());
    }

    #[cfg(windows)]
    const WIN_APP_ID: &str = "dev.formalsnake.ndgatem10win";

    /// Velopack's version of the install in `dir`, from `current\sq.version`.
    #[cfg(windows)]
    fn installed_version(dir: &Path) -> String {
        let nuspec = std::fs::read_to_string(dir.join("current").join("sq.version")).unwrap_or_default();
        nuspec.split("<version>").nth(1).and_then(|v| v.split('<').next()).unwrap_or_default().to_owned()
    }

    /// The app's output so far, read from the agent that launched it.
    #[cfg(windows)]
    fn output(agent: &mut ndshell::agent::Client, pid: i64) -> Vec<String> {
        let reply = agent.call(serde_json::json!({ "cmd": "output", "pid": pid })).unwrap_or_default();
        reply["lines"].as_array().into_iter().flatten().filter_map(|l| l.as_str().map(str::to_owned)).collect()
    }

    /// The Windows leg, through the agent in the logged-in session (Setup.exe and the app need a
    /// desktop): `nd package windows` gives a branded Setup.exe that installs and launches the app;
    /// the installed 0.1.0 takes a signed delta to 0.1.1 from `nd release serve` and applies it
    /// when it closes; a tampered feed is refused before any download.
    #[cfg(windows)]
    #[test]
    fn real_package_windows_then_signed_delta_update() {
        use serde_json::json;

        if std::env::var_os("NEXTEST").is_none() && std::env::var_os("ND_REAL").is_none_or(|v| v != "1") {
            println!("skipped: opens real windows through the agent; run with nextest --profile real or ND_REAL=1");
            return;
        }
        let mut agent = ndshell::agent::Client::connect().expect("the agent (ndshell agent install)");
        agent.set_timeout(Some(Duration::from_secs(60)));
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("nd-m10-win");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let install = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join(WIN_APP_ID);
        let _ = std::fs::remove_dir_all(&install);
        let nd_crate = Path::new(env!("CARGO_MANIFEST_DIR")).join("../nd");

        // nd new demo, branded, nd package windows: Setup.exe installs and launches it.
        nd(&base, &["new", "demo", "--standalone", "--id", WIN_APP_ID, "--nd-path", nd_crate.to_str().unwrap()]);
        let demo = base.join("demo");
        // nd builds into a shared CARGO_TARGET_DIR when one is set, and its feed goes there too.
        let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| demo.join("target"), PathBuf::from);
        let feed = target.join("nd/releases");
        let _ = std::fs::remove_dir_all(&feed);
        let config = std::fs::read_to_string(demo.join("nativedesktop.toml")).unwrap();
        std::fs::write(
            demo.join("nativedesktop.toml"),
            format!(
                "{config}\n[windows]\npublisher = \"FormalSnake\"\naccent = \"#3366ff\"\ninstaller = {{ shortcuts = [\"start\"] }}\n"
            ),
        )
        .unwrap();
        let out = nd(&demo, &["package", "windows"]);
        let setup = out
            .lines()
            .find_map(|l| l.strip_prefix("installer: "))
            .map(PathBuf::from)
            .unwrap_or_else(|| panic!("no Setup.exe: {out}"));
        assert!(setup.exists(), "{}", setup.display());
        agent.call(json!({ "cmd": "launch", "exe": setup.display().to_string() })).expect("run Setup.exe");
        let window = agent
            .call(json!({ "cmd": "window", "title": "Demo", "timeout_ms": 120_000 }))
            .expect("Setup.exe installs and launches the app");
        assert_eq!(installed_version(&install), "0.1.0");
        let shortcut =
            PathBuf::from(std::env::var_os("APPDATA").unwrap()).join("Microsoft/Windows/Start Menu/Programs/Demo.lnk");
        assert!(shortcut.exists(), "the branded Start menu shortcut {}", shortcut.display());
        agent.call(json!({ "cmd": "close", "hwnd": window["hwnd"] })).unwrap();

        // Signing key and feed; 0.1.0 and 0.1.1 released, the second with a delta.
        let port = free_port();
        let key = base.join("signing.key");
        let keygen = nd(&demo, &["release", "keygen", "--out", key.to_str().unwrap()]);
        let config = std::fs::read_to_string(demo.join("nativedesktop.toml")).unwrap();
        std::fs::write(
            demo.join("nativedesktop.toml"),
            format!("{config}\n[updates]\nfeed = \"http://127.0.0.1:{port}\"\n{keygen}"),
        )
        .unwrap();
        nd(&demo, &["release", "--key", key.to_str().unwrap()]);
        let bump = |from: &str, to: &str, label: &str| {
            let cargo = std::fs::read_to_string(demo.join("Cargo.toml")).unwrap();
            std::fs::write(
                demo.join("Cargo.toml"),
                cargo.replacen(&format!("version = \"{from}\""), &format!("version = \"{to}\""), 1),
            )
            .unwrap();
            let app = std::fs::read_to_string(demo.join("src/app.rs")).unwrap();
            std::fs::write(demo.join("src/app.rs"), app.replace("text(\"clicks\")", &format!("text(\"{label}\")")))
                .unwrap();
        };
        bump("0.1.0", "0.1.1", "clicks in 0.1.1");
        let notes = base.join("notes.md");
        std::fs::write(&notes, "The counter label names its version.\n").unwrap();
        nd(&demo, &["release", "--key", key.to_str().unwrap(), "--notes", notes.to_str().unwrap()]);
        let delta = format!("{WIN_APP_ID}-0.1.1-win-stable-delta.nupkg");
        let full_011 = format!("{WIN_APP_ID}-0.1.1-win-stable-full.nupkg");
        assert!(feed.join(&delta).exists(), "no delta in {:?}", nd_cli::package::listing(&feed));

        // The installed 0.1.0 finds 0.1.1, downloads the delta, applies it once it closes.
        let serve = Serve::start(&demo, &feed, port, base.join("serve.log"));
        let exe = install.join("current").join("demo.exe");
        let launched = agent
            .call(json!({ "cmd": "launch", "exe": exe.display().to_string(), "env": { "ND_UPDATE_DELAY_MS": "200" } }))
            .unwrap();
        let pid = launched["pid"].as_i64().unwrap();
        let start = Instant::now();
        loop {
            let lines = output(&mut agent, pid);
            if lines.iter().any(|l| l.contains("update 0.1.1 ready")) {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(90),
                "timed out waiting for update 0.1.1 ready\napp: {lines:?}\nfeed: {:?}",
                serve.requests()
            );
            std::thread::sleep(Duration::from_millis(200));
        }
        let window = agent.call(json!({ "cmd": "window", "pid": pid })).unwrap();
        agent.call(json!({ "cmd": "close", "hwnd": window["hwnd"] })).unwrap();
        wait_until("0.1.1 applied after the app closed", Duration::from_secs(90), || {
            installed_version(&install) == "0.1.1"
        });
        let requests = serve.requests();
        assert!(requests.contains(&"GET /releases.win-stable.json.sig 200".to_string()), "{requests:?}");
        assert!(requests.contains(&format!("GET /{delta} 200")), "{requests:?}");
        assert!(!requests.iter().any(|r| r.contains(&full_011)), "fell back to the full package: {requests:?}");

        // 0.1.2 released, then its feed edited under the same signature: refused before any download.
        bump("0.1.1", "0.1.2", "clicks in 0.1.2");
        nd(&demo, &["release", "--key", key.to_str().unwrap()]);
        let json_path = feed.join("releases.win-stable.json");
        let text = std::fs::read_to_string(&json_path).unwrap();
        std::fs::write(&json_path, text.replace("0.1.2", "0.1.3")).unwrap();
        let before = serve.requests().len();
        let launched = agent
            .call(json!({ "cmd": "launch", "exe": exe.display().to_string(), "env": { "ND_UPDATE_DELAY_MS": "200" } }))
            .unwrap();
        let pid = launched["pid"].as_i64().unwrap();
        wait_until("the update check", Duration::from_secs(60), || {
            output(&mut agent, pid).iter().any(|l| l.contains("nd: update") || l.contains("does not match"))
        });
        let line = output(&mut agent, pid).into_iter().find(|l| l.contains("does not match"));
        assert!(line.is_some(), "no refusal: {:?}", output(&mut agent, pid));
        let after = serve.requests();
        assert!(after[before..].iter().all(|r| !r.contains(".nupkg")), "downloaded from a tampered feed: {after:?}");
        let _ = agent.call(json!({ "cmd": "kill", "pid": pid }));
        assert_eq!(installed_version(&install), "0.1.1");
        drop(serve);
    }
}
