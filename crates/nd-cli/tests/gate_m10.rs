//! M10 gate: `nd new` + `nd package mac` gives a launching `.app`; 0.1.0 updates to 0.1.1
//! through `nd release serve` with a signed delta, applied on quit; a tampered feed is refused;
//! every publisher writes to a local target.

mod gate_m10 {
    #[cfg(target_os = "macos")]
    use std::path::{Path, PathBuf};
    #[cfg(target_os = "macos")]
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex};
    #[cfg(target_os = "macos")]
    use std::time::{Duration, Instant};

    use nd_cli::publish::{self, GhCommand, S3Target};
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

    #[cfg(target_os = "macos")]
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

    #[cfg(target_os = "macos")]
    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    #[cfg(target_os = "macos")]
    struct Serve {
        child: Child,
        log: PathBuf,
    }

    #[cfg(target_os = "macos")]
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

    #[cfg(target_os = "macos")]
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

    #[cfg(target_os = "macos")]
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
}
