//! M1 automation gate (ARCHITECTURE.md 16): `just gate m1-automation`.
//!
//! - `tier1_locator_click`: a locator clicks a button in an in-process app and `expect` sees the
//!   label change.
//! - `real_locator_clicks_hello` (tier 3): the same against `examples/hello` as its own process,
//!   over the automation socket, clicking the real `NSButton` through its target/action.
//! - `real_lock_queues` (tier 3): two concurrent runs of the hello test queue on the machine-wide
//!   lock; their lock-holding intervals do not overlap and the second says whom it waited for.
//!
//! Tier 3 tests need `hello` built (`cargo build --workspace --bins`) and run only under nextest
//! or with `ND_REAL=1`.

mod common;

fn real_allowed(name: &str) -> bool {
    let allowed = std::env::var_os("NEXTEST").is_some() || std::env::var_os("ND_REAL").is_some_and(|v| v == "1");
    if !allowed {
        println!("{name}: skipped, it opens real windows; run with nextest --profile real or ND_REAL=1");
    }
    allowed
}

fn now_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()
}

mod gate_m1_automation {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use nd_test::{LaunchOptions, Role, expect, launch_app};

    use super::{common, now_ms, real_allowed};

    #[test]
    fn tier1_locator_click() {
        let app = common::harness();
        let count = app.get_by_test_id("result").get_by_role(Role::Text).first();
        expect(&count).to_have_text("0").unwrap();
        app.get_by_role(Role::Button).name("Add one").click().unwrap();
        expect(&count).to_have_text("1").unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn real_locator_clicks_hello() {
        if !real_allowed("gate_m1_automation::real_locator_clicks_hello") {
            return;
        }
        let app = launch_app(LaunchOptions::example("hello")).unwrap();
        println!("ND_GATE_LOCKED {}", now_ms());
        let count = app.locator("role=text >> nth=0");
        expect(&count).to_have_text("0").unwrap();
        let add = app.get_by_role(Role::Button).name("Add one").exact();
        assert_eq!(add.node().unwrap().kind, "Button");
        let started = Instant::now();
        add.click().unwrap();
        expect(&count).to_have_text("1").unwrap();
        println!("click to label change: {:?}", started.elapsed());
        app.get_by_test_id("add").click().unwrap();
        expect(&app.get_by_text("2").exact()).to_be_visible().unwrap();

        let shot = app.dir().join("hello.png");
        let reply = app.screenshot(&shot, None).unwrap();
        assert!(reply["width"].as_u64().unwrap() > 0 && std::fs::metadata(&shot).unwrap().len() > 0, "{reply}");

        if let Some(ms) = std::env::var("ND_GATE_HOLD_MS").ok().and_then(|v| v.parse().ok()) {
            std::thread::sleep(Duration::from_millis(ms));
        }
        println!("ND_GATE_RELEASED {}", now_ms());
        drop(app);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn real_lock_queues() {
        if !real_allowed("gate_m1_automation::real_lock_queues") {
            return;
        }
        let exe = std::env::current_exe().unwrap();
        let run = || {
            Command::new(&exe)
                .args(["--exact", "gate_m1_automation::real_locator_clicks_hello", "--nocapture", "--test-threads=1"])
                .env("ND_REAL", "1")
                .env_remove("NEXTEST")
                .env("ND_GATE_HOLD_MS", "1500")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        };
        let first = run();
        // Start the second only once the first holds the lock, so the order is known.
        let start = Instant::now();
        let first_pid = format!("pid={} ", first.id());
        while !ndshell::lock::holder().starts_with(&first_pid) {
            assert!(start.elapsed() < Duration::from_secs(180), "the first run never took the lock");
            std::thread::sleep(Duration::from_millis(20));
        }
        let second = run();
        let (first, second) = (first.wait_with_output().unwrap(), second.wait_with_output().unwrap());
        let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        let stamp = |out: &str, key: &str| -> u128 {
            // libtest prints `test <name> ... ` on the same line as the test's first output.
            let at = out.find(key).unwrap_or_else(|| panic!("no {key} in\n{out}"));
            out[at + key.len()..].split_whitespace().next().unwrap().parse().unwrap()
        };
        let (out1, out2) = (text(&first.stdout), text(&second.stdout));
        let (err1, err2) = (text(&first.stderr), text(&second.stderr));
        assert!(first.status.success(), "first run failed\n{out1}\n{err1}");
        assert!(second.status.success(), "second run failed\n{out2}\n{err2}");
        let (released1, locked2) = (stamp(&out1, "ND_GATE_RELEASED"), stamp(&out2, "ND_GATE_LOCKED"));
        assert!(released1 <= locked2, "the runs overlapped: first released at {released1}, second locked at {locked2}");
        let waited = err2.lines().find(|l| l.contains("waiting for the real-input lock"));
        let waited = waited.unwrap_or_else(|| panic!("the second run did not report waiting\n{err2}"));
        assert!(waited.contains(&first_pid), "the waiting run names the holder: {waited}");
        println!("second run waited {} ms; {waited}", locked2 - stamp(&out1, "ND_GATE_LOCKED"));
    }
}
