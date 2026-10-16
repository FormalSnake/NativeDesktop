//! A headless session leaves the user's systemd and D-Bus activation environment alone, while it
//! runs and after it stops, and leaves no process behind. Tier 3: it starts the real compositors.
#![cfg(target_os = "linux")]

use std::process::Command;
use std::time::Duration;

/// `systemctl --user show-environment`, or `None` without a user manager (CI containers).
fn user_environment() -> Option<String> {
    let out = Command::new("systemctl").args(["--user", "show-environment"]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Hyprland and labwc spawn their activation-environment updates asynchronously, so give them
/// time to land before comparing.
fn settled() -> String {
    std::thread::sleep(Duration::from_millis(1500));
    user_environment().expect("systemctl --user show-environment")
}

fn assert_untouched<T>(name: &str, start: impl FnOnce() -> T) {
    let Some(before) = user_environment() else {
        eprintln!("skip: no systemd user manager");
        return;
    };
    let session = start();
    let during = settled();
    drop(session);
    let after = settled();
    assert_eq!(before, during, "a running {name} changed the user manager's environment");
    assert_eq!(before, after, "stopping {name} changed the user manager's environment");
}

/// Pids whose process group is `pgid`.
fn group_members(pgid: u32) -> Vec<u32> {
    let procs = std::fs::read_dir("/proc").expect("/proc");
    procs
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            // `pid (comm) state ppid pgrp ...`; comm may hold spaces, so split after its paren.
            let rest = stat.rsplit_once(')').map_or("", |(_, rest)| rest);
            rest.split_whitespace().nth(2) == Some(pgid.to_string().as_str())
        })
        .collect()
}

#[test]
fn real_hyprland_keeps_out_of_the_user_manager() {
    let mut pid = 0;
    assert_untouched("headless Hyprland", || {
        let hyprland = ndshell::hyprland::Hyprland::start(Default::default()).expect("headless Hyprland");
        pid = hyprland.pid();
        hyprland
    });
    if pid != 0 {
        assert_eq!(group_members(pid), Vec::<u32>::new(), "Hyprland's process group outlived it");
    }
}

#[test]
fn real_weston_keeps_out_of_the_user_manager() {
    assert_untouched("headless Weston", || ndshell::weston::Weston::start(800, 600).expect("headless Weston"));
}
