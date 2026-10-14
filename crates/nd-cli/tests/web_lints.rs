//! The web lint table against the framework's real sources: the markers `nd check --target web`
//! reads are where the docs say, and an unguarded call is reported while a guarded one is not.

use std::path::{Path, PathBuf};

use nd_cli::lints::{lint_source_for, unavailable_on_web};

fn framework() -> Vec<PathBuf> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("crates dir");
    ["nd", "nd-components"].iter().map(|c| crates.join(c).join("src")).collect()
}

#[test]
fn markers_in_the_framework_are_found() {
    let names = unavailable_on_web(&framework()).expect("scan");
    assert!(names.contains("report_issue"), "{names:?}");
    assert!(names.contains("directory"), "{names:?}");
}

#[test]
fn an_unguarded_call_is_reported_and_a_guarded_one_is_not() {
    let names = unavailable_on_web(&framework()).expect("scan");
    let unguarded = "fn f() { let _ = nd::report_issue(\"stuck\"); }";
    let findings = lint_source_for(Path::new("src/app.rs"), unguarded, Some(&names));
    assert!(findings.iter().any(|f| f.rule == "web-unavailable"), "{findings:?}");
    let guarded = "fn f() { if nd::platform::has(Feature::FileSystem) { let _ = nd::report_issue(\"stuck\"); } }";
    let findings = lint_source_for(Path::new("src/app.rs"), guarded, Some(&names));
    assert!(findings.iter().all(|f| f.rule != "web-unavailable"), "{findings:?}");
}
