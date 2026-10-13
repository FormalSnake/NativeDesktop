//! The real-input lock between two holders, on a private lock directory.

use ndshell::lock;

fn dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ndshell-lock-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn second_holder_waits_until_the_first_lets_go() {
    let dir = dir("queue");
    let first = lock::acquire_in(&dir, "first").unwrap();
    let held = lock::holder_in(&dir);
    assert!(held.contains(&format!("pid={}", std::process::id())) && held.contains("purpose=first"), "{held}");
    assert!(lock::try_acquire_in(&dir, "second").unwrap().is_none(), "the lock is exclusive");

    let waiter = {
        let dir = dir.clone();
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let lock = lock::acquire_in(&dir, "second").unwrap();
            (start.elapsed(), lock)
        })
    };
    std::thread::sleep(std::time::Duration::from_millis(300));
    drop(first);
    let (waited, second) = waiter.join().unwrap();
    assert!(waited >= std::time::Duration::from_millis(250), "the second holder queued ({waited:?})");
    assert!(lock::holder_in(&dir).contains("purpose=second"));
    drop(second);
    assert_eq!(lock::holder_in(&dir), "", "a released lock names no holder");
    let _ = std::fs::remove_dir_all(&dir);
}
