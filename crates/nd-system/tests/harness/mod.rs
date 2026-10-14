//! The libtest command line, enough for `cargo test` and nextest's custom harness contract, for
//! this crate's `harness = false` binaries (AppKit and AVPlayer need the main thread). Each binary
//! holds one test: `--list` names it, `--list --ignored` names it when it is ignored, a run takes
//! filters (substring, or whole name with `--exact`) and runs an ignored test only with
//! `--ignored` or `--include-ignored`.

pub struct Test {
    pub name: &'static str,
    /// Whether this binary's test exists on the running OS.
    pub here: bool,
    /// Left out of default runs; a gate script asks for it with `--ignored`.
    pub ignored: bool,
}

/// Lists or runs the test; `run` returns when it passed and panics when it failed.
pub fn main(test: Test, run: fn()) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let (ignored_only, include_ignored) = (has("--ignored"), has("--include-ignored"));
    if has("--list") {
        if test.here && test.ignored == ignored_only {
            println!("{}: test", test.name);
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
    let selected = filters.is_empty()
        || filters.iter().any(|f| if exact { *f == test.name } else { test.name.contains(f.as_str()) });
    let wanted = if test.ignored { ignored_only || include_ignored } else { !ignored_only };
    if !test.here || !selected || !wanted {
        println!("running 0 tests");
        return;
    }
    run();
    println!("test {} ... ok", test.name);
}
