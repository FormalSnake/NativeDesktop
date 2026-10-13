//! `nd diagnose`: reads the incident bundles `nd-diag` wrote for the app in the current project
//! (ARCHITECTURE.md 9.3, 9.6) and prints what an agent needs before reproducing a bug.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};
use clap::Args;
use nd_core::semantic::{SemanticTree, SemanticWindow};
use nd_diag::BundleInfo;
use serde_json::{Value, json};

const INPUTS: usize = 10;
const FRAMES: usize = 5;
const LOG_TAIL: usize = 15;
const TOP_FRAMES: usize = 6;

#[derive(Args, Debug)]
pub struct DiagnoseArgs {
    /// Bundle id or a unique part of it, as `--list` prints it.
    pub id: Option<String>,
    /// Summarise the newest bundle. The default when no id is given.
    #[arg(long, conflicts_with_all = ["id", "list"])]
    pub last: bool,
    /// List bundles, newest first.
    #[arg(long, conflicts_with = "id")]
    pub list: bool,
    /// Machine-readable output.
    #[arg(long)]
    pub json: bool,
    /// The app's name (its Cargo package name) when the current directory does not say.
    #[arg(long)]
    pub app: Option<String>,
    #[arg(long)]
    pub manifest_path: Option<PathBuf>,
}

fn app_name(args: &DiagnoseArgs) -> Result<String> {
    if let Some(app) = &args.app {
        return Ok(app.clone());
    }
    let mut cmd = Command::new("cargo");
    cmd.args(["metadata", "--no-deps", "--format-version", "1"]);
    if let Some(path) = &args.manifest_path {
        cmd.arg("--manifest-path").arg(path);
    }
    let output = cmd.output().context("running cargo metadata")?;
    if !output.status.success() {
        bail!("not in a Cargo project (pass --app <name>): {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    let meta: Value = serde_json::from_slice(&output.stdout)?;
    let manifest = match &args.manifest_path {
        Some(p) => fs::canonicalize(p).ok(),
        None => std::env::current_dir().ok().map(|d| d.join("Cargo.toml")),
    };
    let packages = meta["packages"].as_array().map_or(&[][..], Vec::as_slice);
    let name_of = |pkg: &Value| pkg["name"].as_str().map(str::to_owned);
    if let Some(here) = manifest.as_deref().and_then(Path::to_str)
        && let Some(name) = packages.iter().find(|p| p["manifest_path"] == here).and_then(name_of)
    {
        return Ok(name);
    }
    let defaults: Vec<&str> =
        meta["workspace_default_members"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let mut members = packages.iter().filter(|p| p["id"].as_str().is_some_and(|id| defaults.contains(&id)));
    match (members.next().and_then(name_of), members.next()) {
        (Some(name), None) => Ok(name),
        _ => bail!("cannot tell which app this is (pass --app <name>)"),
    }
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn read_lines(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .map(|text| text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
        .unwrap_or_default()
}

fn tail(mut items: Vec<Value>, n: usize) -> Vec<Value> {
    items.drain(..items.len().saturating_sub(n));
    items
}

/// Frames worth reading: the backtrace without the runtime and panic machinery above and below
/// the app's own code.
fn top_frames(backtrace: &str) -> Vec<String> {
    const NOISE: &[&str] = &[
        "std::",
        "core::",
        "alloc::",
        "<std::",
        "<core::",
        "<alloc::",
        "backtrace::",
        "nd_diag::",
        "rust_begin_unwind",
        "__rust",
        "_main",
        "start",
        "<unknown>",
    ];
    let mut out = Vec::new();
    let mut lines = backtrace.lines().peekable();
    while let Some(line) = lines.next() {
        let Some((index, name)) = line.trim().split_once(": ") else { continue };
        if index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let at = lines.next_if(|l| l.trim_start().starts_with("at ")).map(|l| l.trim_start()[3..].to_owned());
        if NOISE.iter().any(|n| name.starts_with(n)) {
            continue;
        }
        out.push(match at {
            Some(at) => format!("{name} ({at})"),
            None => name.to_owned(),
        });
        if out.len() == TOP_FRAMES {
            break;
        }
    }
    out
}

fn panic_summary(text: &str) -> Value {
    let (head, rest) = text.split_once("\n\n").unwrap_or((text, ""));
    let (first, message) = head.split_once('\n').unwrap_or((head, ""));
    let thread = first.strip_prefix("thread '").and_then(|s| s.split_once('\'')).map(|(t, _)| t);
    let location = first.split_once("panicked at ").map(|(_, l)| l.trim_end_matches(':'));
    json!({ "thread": thread, "location": location, "message": message, "frames": top_frames(rest) })
}

fn frame_summary(profile: &Value) -> Value {
    let windows: serde_json::Map<String, Value> = profile["windows"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(id, w)| {
            let frames = w["frames"].as_array().cloned().unwrap_or_default();
            (id.clone(), json!({ "summary": w["summary"], "last": tail(frames, FRAMES) }))
        })
        .collect();
    Value::Object(windows)
}

/// Everything `--last` shows, as one value: the text output renders it, `--json` prints it.
pub fn load(bundle: &BundleInfo) -> Value {
    let dir = &bundle.path;
    let meta = read_json(&dir.join("meta.json")).unwrap_or(Value::Null);
    let mut incident = json!({ "kind": bundle.kind });
    for file in ["panic.txt", "report.txt"] {
        if let Ok(text) = fs::read_to_string(dir.join(file)) {
            incident["file"] = file.into();
            if file == "panic.txt" {
                incident["panic"] = panic_summary(&text);
            } else {
                incident["message"] = text.into();
            }
        }
    }
    for file in ["hang.json", "crash.json"] {
        if let Some(detail) = read_json(&dir.join(file)) {
            incident["file"] = file.into();
            incident["detail"] = detail;
        }
    }
    let tree_json = read_json(&dir.join("tree.json")).unwrap_or(Value::Null);
    let windows: Vec<SemanticWindow> = serde_json::from_value(tree_json["windows"].clone()).unwrap_or_default();
    let tree = SemanticTree { windows };
    json!({
        "id": bundle.id,
        "path": dir,
        "kind": bundle.kind,
        "age_seconds": bundle.age.map(|a| a.as_secs()),
        "meta": meta,
        "incident": incident,
        "minidump": dir.join("minidump.dmp").exists(),
        "tree": tree.snapshot(),
        "inputs": tail(read_lines(&dir.join("input.ndjson")), INPUTS),
        "frames": frame_summary(&read_json(&dir.join("profile.json")).unwrap_or(Value::Null)),
        "log": tail(read_lines(&dir.join("log.ndjson")), LOG_TAIL),
    })
}

pub fn age(seconds: Option<u64>) -> String {
    match seconds {
        None => "unknown age".into(),
        Some(s) if s < 90 => format!("{s}s ago"),
        Some(s) if s < 90 * 60 => format!("{}m ago", s / 60),
        Some(s) if s < 36 * 3600 => format!("{}h ago", s / 3600),
        Some(s) => format!("{}d ago", s / 86400),
    }
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("?")
}

fn f1(v: &Value) -> String {
    v.as_f64().map_or("?".into(), |n| format!("{n:.1}"))
}

/// The compact text form of [`load`].
pub fn render(report: &Value) -> String {
    let mut out = String::new();
    let meta = &report["meta"];
    let _ = writeln!(out, "{} ({}, {})", s(&report["id"]), s(&report["kind"]), age(report["age_seconds"].as_u64()));
    let _ = writeln!(out, "path: {}", s(&report["path"]));
    let _ = writeln!(
        out,
        "app: {} {} on {} {} {} ({} build, pid {})",
        s(&meta["app"]),
        s(&meta["version"]),
        s(&meta["backend"]),
        s(&meta["os"]),
        s(&meta["os_version"]),
        if meta["debug"] == true { "debug" } else { "release" },
        meta["pid"],
    );

    let incident = &report["incident"];
    let _ = writeln!(out, "\nwhat happened");
    if let Some(p) = incident.get("panic") {
        let _ = writeln!(out, "panic on thread '{}' at {}", s(&p["thread"]), s(&p["location"]));
        let _ = writeln!(out, "{}", s(&p["message"]));
        for frame in p["frames"].as_array().into_iter().flatten() {
            let _ = writeln!(out, "  at {}", s(frame));
        }
    } else if let Some(m) = incident.get("message") {
        let _ = writeln!(out, "user report: {}", s(m).trim());
    } else if let Some(d) = incident.get("detail") {
        let _ = writeln!(out, "{}: {d}", s(&report["kind"]));
    } else {
        let _ = writeln!(out, "{}, no detail file", s(&report["kind"]));
    }
    if report["minidump"] == true {
        let _ = writeln!(out, "minidump: {}/minidump.dmp", s(&report["path"]));
    }

    let _ = writeln!(out, "\nsemantic tree at the last frame");
    let tree = s(&report["tree"]);
    out.push_str(if tree.is_empty() { "(none recorded)\n" } else { tree });

    let _ = writeln!(out, "\nlast inputs");
    let inputs = report["inputs"].as_array().map_or(&[][..], Vec::as_slice);
    if inputs.is_empty() {
        let _ = writeln!(out, "(none)");
    }
    for i in inputs {
        let at = i["position"].as_array().map_or(String::new(), |p| {
            format!(" at {:.0},{:.0}", p[0].as_f64().unwrap_or(0.), p[1].as_f64().unwrap_or(0.))
        });
        let text = i["text"].as_str().map_or(String::new(), |t| format!(" {t:?}"));
        let _ = writeln!(out, "{} window {}{at}{text}", s(&i["kind"]), i["window"]);
    }

    let _ = writeln!(out, "\nframe timings (ms)");
    let windows = report["frames"].as_object();
    if windows.is_none_or(|w| w.is_empty()) {
        let _ = writeln!(out, "(none)");
    }
    for (id, w) in windows.into_iter().flatten() {
        let sum = &w["summary"];
        let _ = writeln!(
            out,
            "window {id}: {} frames, {} missed, work p50 {} p95 {} max {}",
            sum["frames"],
            sum["missed"],
            f1(&sum["work_ms_p50"]),
            f1(&sum["work_ms_p95"]),
            f1(&sum["work_ms_max"]),
        );
        let last: Vec<String> = w["last"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|f| format!("{}{}", f1(&f["work_ms"]), if f["missed"] == true { "!" } else { "" }))
            .collect();
        let _ = writeln!(out, "  last: {} (! = missed)", last.join(" "));
    }

    let _ = writeln!(out, "\nlog tail");
    for l in report["log"].as_array().into_iter().flatten() {
        let _ = writeln!(out, "{:<5} {}: {}", s(&l["level"]), s(&l["target"]), s(&l["message"]));
    }
    out
}

fn list_text(bundles: &[BundleInfo]) -> String {
    bundles.iter().map(|b| format!("{}  {:<6}  {}\n", b.id, b.kind, age(b.age.map(|a| a.as_secs())))).collect()
}

fn pick<'a>(bundles: &'a [BundleInfo], id: &str) -> Result<&'a BundleInfo> {
    if let Some(exact) = bundles.iter().find(|b| b.id == id) {
        return Ok(exact);
    }
    let mut matches = bundles.iter().filter(|b| b.id.contains(id));
    match (matches.next(), matches.next()) {
        (Some(b), None) => Ok(b),
        (None, _) => bail!("no bundle matches `{id}` (see `nd diagnose --list`)"),
        _ => bail!("`{id}` matches more than one bundle (see `nd diagnose --list`)"),
    }
}

/// The output for `args` against the bundles in `dir`.
pub fn output(args: &DiagnoseArgs, dir: &Path) -> Result<String> {
    let bundles = nd_diag::list_bundles(dir).with_context(|| format!("reading {}", dir.display()))?;
    if args.list {
        return Ok(if args.json {
            let rows: Vec<Value> = bundles
                .iter()
                .map(|b| json!({ "id": b.id, "kind": b.kind, "age_seconds": b.age.map(|a| a.as_secs()), "path": b.path }))
                .collect();
            serde_json::to_string_pretty(&rows)? + "\n"
        } else if bundles.is_empty() {
            format!("no incident bundles in {}\n", dir.display())
        } else {
            list_text(&bundles)
        });
    }
    let bundle = match &args.id {
        Some(id) => pick(&bundles, id)?,
        None => match bundles.first() {
            Some(b) => b,
            None if args.json => return Ok("null\n".into()),
            None => return Ok(format!("no incident bundles in {}\n", dir.display())),
        },
    };
    let report = load(bundle);
    Ok(if args.json { serde_json::to_string_pretty(&report)? + "\n" } else { render(&report) })
}

pub fn run(args: DiagnoseArgs) -> Result<ExitCode> {
    let dir = nd_diag::diagnostics_dir(&app_name(&args)?);
    print!("{}", output(&args, &dir)?);
    Ok(ExitCode::SUCCESS)
}
