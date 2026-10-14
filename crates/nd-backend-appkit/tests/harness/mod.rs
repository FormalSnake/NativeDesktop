//! The libtest command line, enough for `cargo test` and nextest's custom harness contract, for
//! test binaries that must own the main thread (`harness = false`, AppKit needs it). Each binary
//! holds exactly one test.

#[cfg(target_os = "macos")]
#[allow(dead_code)]
pub mod resize;

/// As [`main`] for a tier 3 test, which takes over the real cursor: it runs only under nextest
/// (whose profiles keep `real_*` tests out of default runs) or with `ND_REAL=1`.
#[allow(dead_code)]
pub fn main_real(name: &str, run: fn()) {
    let listing = std::env::args().any(|a| a == "--list");
    let allowed = std::env::var_os("NEXTEST").is_some() || std::env::var_os("ND_REAL").is_some_and(|v| v == "1");
    if !listing && !allowed {
        println!("{name}: skipped, it moves the real cursor; run with nextest --profile real or ND_REAL=1");
        return;
    }
    main(name, run);
}

/// Lists or runs `name`. `run` exits the process itself with the test's status.
pub fn main(name: &str, run: fn()) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let ignored_only = has("--ignored");
    if has("--list") {
        if !ignored_only && cfg!(target_os = "macos") {
            println!("{name}: test");
        }
        return;
    }
    let exact = has("--exact");
    let mut skip_value = false;
    let filters: Vec<&String> = args
        .iter()
        .filter(|a| {
            if std::mem::take(&mut skip_value) {
                return false;
            }
            // Flags taking a value; the value is not a filter.
            if matches!(a.as_str(), "--format" | "--test-threads" | "--color" | "--skip" | "-Z") {
                skip_value = true;
            }
            !a.starts_with('-')
        })
        .collect();
    let selected =
        filters.is_empty() || filters.iter().any(|f| if exact { *f == name } else { name.contains(f.as_str()) });
    if ignored_only || !selected || !cfg!(target_os = "macos") {
        return;
    }
    run();
}
