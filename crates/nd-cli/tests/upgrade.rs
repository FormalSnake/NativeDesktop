//! Tier 1: `nd new` depends on nd by release tag and `nd upgrade` moves it, against a local bare
//! repository standing in for the GitHub remote. Its nd and GPUI crates are empty stand-ins, so
//! only resolution runs, never a build.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn check(output: Output, what: &str) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(output.status.success(), "{what} failed\n{stdout}\n{}", String::from_utf8_lossy(&output.stderr));
    stdout
}

fn package(name: &str, version: &str, features: &str) -> String {
    format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\nedition = \"2024\"\n\n[features]\n{features}")
}

/// One fast-import commit of `files` on main, tagged `tag`. fast-import takes the committer from
/// the stream, so the fixture needs no git identity on the machine.
fn commit(stream: &mut String, mark: usize, tag: &str, files: &[(String, String)]) {
    stream.push_str(&format!("commit refs/heads/main\nmark :{mark}\n"));
    stream.push_str("committer nd-test <nd-test@example.invalid> 0 +0000\n");
    stream.push_str(&format!("data {}\n{tag}\n", tag.len() + 1));
    if mark > 1 {
        stream.push_str(&format!("from :{}\n", mark - 1));
    }
    for (path, text) in files {
        stream.push_str(&format!("M 100644 inline {path}\ndata {}\n{text}\n", text.len()));
    }
    stream.push_str(&format!("reset refs/tags/{tag}\nfrom :{mark}\n\n"));
}

fn release(version: &str, changelog: &str) -> Vec<(String, String)> {
    let mut files = vec![
        ("CHANGELOG.md".to_string(), changelog.to_string()),
        (
            "crates/nd/Cargo.toml".to_string(),
            package("nd", version, "default = [\"runtime\"]\nruntime = []\nbuild = []\ntest = []\naudit = []\n"),
        ),
        ("crates/nd/src/lib.rs".to_string(), String::new()),
    ];
    for krate in nd_cli::new::gpui_patch_crates() {
        let dir = format!("crates/nd-gpui/vendor/{krate}");
        files.push((format!("{dir}/Cargo.toml"), package(krate, "0.3.9", "")));
        files.push((format!("{dir}/src/lib.rs"), String::new()));
    }
    files
}

fn minor_bump(version: &str) -> String {
    let mut parts = version.split('.').map(|p| p.parse::<u64>().expect("numeric version"));
    let (major, minor) = (parts.next().expect("major"), parts.next().expect("minor"));
    format!("{major}.{}.0", minor + 1)
}

#[test]
fn nd_new_depends_on_the_release_tag_and_nd_upgrade_moves_it() {
    let nd = env!("CARGO_BIN_EXE_nd");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let dir = tempfile::tempdir().expect("temp dir");
    let cargo_home = dir.path().join("cargo-home");
    let remote = dir.path().join("nd.git");
    let url = url::Url::from_file_path(&remote).expect("file url").to_string();

    let current = env!("CARGO_PKG_VERSION");
    let next = minor_bump(current);
    let (from, to) = (format!("v{current}"), format!("v{next}"));
    let first = format!("# Changelog\n\n## {from}\n\n- first\n");
    let second = format!("# Changelog\n\n## {to}\n\n- second\n\n## {from}\n\n- first\n");
    let mut stream = String::new();
    commit(&mut stream, 1, &from, &release(current, &first));
    commit(&mut stream, 2, &to, &release(&next, &second));
    check(Command::new("git").args(["init", "-q", "--bare"]).arg(&remote).output().expect("git init"), "git init");
    let mut import = Command::new("git")
        .args(["fast-import", "--quiet"])
        .current_dir(&remote)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("git fast-import");
    import.stdin.take().expect("stdin").write_all(stream.as_bytes()).expect("write stream");
    check(import.wait_with_output().expect("fast-import"), "git fast-import");

    let app = dir.path().join("app");
    let nd_cmd = |args: &[&str]| {
        let mut command = Command::new(nd);
        command.args(args).current_dir(dir.path()).env("CARGO_HOME", &cargo_home).env("CARGO", &cargo);
        check(command.output().expect("nd"), &format!("nd {args:?}"))
    };
    nd_cmd(&["new", "app", "--standalone", "--nd-git", &url]);
    let manifest = std::fs::read_to_string(app.join("Cargo.toml")).expect("Cargo.toml");
    assert!(manifest.contains(&format!("nd = {{ git = \"{url}\", tag = \"{from}\" }}")), "{manifest}");
    let patches = nd_cli::new::gpui_patch_crates().len();
    assert_eq!(manifest.matches(&format!("tag = \"{from}\"")).count(), 3 + patches, "{manifest}");

    let resolved = Command::new(&cargo)
        .args(["metadata", "--format-version", "1"])
        .current_dir(&app)
        .env("CARGO_HOME", &cargo_home)
        .output()
        .expect("cargo metadata");
    check(resolved, "cargo metadata on the new app");
    let lock = std::fs::read_to_string(app.join("Cargo.lock")).expect("Cargo.lock");
    assert!(lock.contains(&format!("?tag={from}#")), "{lock}");

    let out = nd_cmd(&["upgrade", "--path", "app"]);
    assert!(out.contains(&format!("nd {from} -> {to}")), "{out}");
    assert!(out.contains(&format!("## {to}\n\n- second")), "{out}");
    assert!(!out.contains("- first"), "{out}");
    let manifest = std::fs::read_to_string(app.join("Cargo.toml")).expect("Cargo.toml");
    assert_eq!(manifest.matches(&format!("tag = \"{to}\"")).count(), 3 + patches, "{manifest}");
    let lock = std::fs::read_to_string(app.join("Cargo.lock")).expect("Cargo.lock");
    assert!(lock.contains(&format!("?tag={to}#")) && !lock.contains(&format!("?tag={from}#")), "{lock}");

    let out = nd_cmd(&["upgrade", "--path", "app"]);
    assert!(out.contains(&format!("already at {to}")), "{out}");
    let out = nd_cmd(&["upgrade", "--path", "app", "--to", &from]);
    assert!(out.contains(&format!("nd {to} -> {from}")), "{out}");
    assert!(out.contains("no CHANGELOG entries"), "{out}");
}

#[test]
fn nd_upgrade_refuses_a_project_without_a_git_nd() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("Cargo.toml"), "[dependencies]\nnd = { path = '../nd' }\n").expect("write");
    let output = Command::new(env!("CARGO_BIN_EXE_nd"))
        .args(["upgrade", "--path"])
        .arg(dir.path())
        .output()
        .expect("nd");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no `nd = { git"), "{output:?}");
    assert!(Path::new(&dir.path().join("Cargo.toml")).is_file());
}
