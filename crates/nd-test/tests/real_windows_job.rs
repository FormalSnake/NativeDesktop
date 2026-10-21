//! Windows has no parent-death signal: an app launched through `launch_app` whose `AppHandle` is
//! leaked must still end with the process that launched it (the job object in `launch.rs`).
//! Needs `hello` built (`cargo build -p hello`).

#![cfg(windows)]

use std::process::Command;
use std::time::{Duration, Instant};

use nd_test::{LaunchOptions, example_bin, launch_app};

const CHILD: &str = "ND_TEST_JOB_CHILD";

fn alive(pid: u32) -> bool {
    let out = Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/NH"]).output().expect("tasklist");
    String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
}

#[test]
fn real_leaked_app_ends_with_its_launcher() {
    if std::env::var_os(CHILD).is_some() {
        let app = launch_app(LaunchOptions::example("hello")).expect("launch hello");
        println!("hello pid={}", app.pid());
        std::mem::forget(app);
        return;
    }
    if !example_bin("hello").exists() {
        println!("skipped: build hello first");
        return;
    }
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "real_leaked_app_ends_with_its_launcher", "--nocapture", "--test-threads", "1"])
        .env(CHILD, "1")
        .output()
        .expect("run the launcher");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let pid: u32 = stdout
        .lines()
        .find_map(|l| l.split_once("hello pid=").map(|(_, p)| p.trim().parse().expect("pid")))
        .unwrap_or_else(|| panic!("the launcher printed no pid:\n{stdout}{}", String::from_utf8_lossy(&out.stderr)));
    let start = Instant::now();
    while alive(pid) {
        assert!(start.elapsed() < Duration::from_secs(5), "hello {pid} outlived the process that launched it");
        std::thread::sleep(Duration::from_millis(100));
    }
}
