//! Incident bundles (ARCHITECTURE.md 9.3) and their retention (9.6).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::record::{FrameRecord, InputRecord};

/// Everything a bundle carries besides its primary file. The crash helper reads the last one the
/// app wrote to disk, since a crashed process cannot write its own.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub meta: Value,
    pub log: Vec<Value>,
    pub trees: Vec<Value>,
    /// The native view and layer hierarchy, where the backend has one (AppKit).
    pub native: Option<Value>,
    pub inputs: Vec<InputRecord>,
    pub frames: BTreeMap<u64, Vec<FrameRecord>>,
    pub layout: Vec<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Panic,
    Crash,
    Hang,
    Report,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Panic => "panic",
            Kind::Crash => "crash",
            Kind::Hang => "hang",
            Kind::Report => "report",
        }
    }
}

/// Creates `<dir>/<utc timestamp>-<kind>/`. Names sort by time; a clash moves the time on by a
/// microsecond rather than adding a counter, so they still do.
pub fn create_dir(dir: &Path, kind: Kind) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let mut now = chrono::Utc::now();
    loop {
        let path = dir.join(format!("{}-{}", now.format("%Y%m%dT%H%M%S%.6fZ"), kind.as_str()));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => now += chrono::Duration::microseconds(1),
            Err(e) => return Err(e),
        }
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let mut out = BufWriter::new(File::create(path)?);
    serde_json::to_writer_pretty(&mut out, value)?;
    out.flush()
}

fn write_lines<T: Serialize>(path: &Path, items: impl IntoIterator<Item = T>) -> io::Result<()> {
    let mut out = BufWriter::new(File::create(path)?);
    for item in items {
        serde_json::to_writer(&mut out, &item)?;
        out.write_all(b"\n")?;
    }
    out.flush()
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn profile(frames: &BTreeMap<u64, Vec<FrameRecord>>) -> Value {
    let windows: serde_json::Map<String, Value> = frames
        .iter()
        .map(|(id, frames)| {
            let mut work: Vec<f64> = frames.iter().map(|f| f.work_ms).collect();
            work.sort_by(f64::total_cmp);
            let summary = json!({
                "frames": frames.len(),
                "missed": frames.iter().filter(|f| f.missed).count(),
                "work_ms_p50": percentile(&work, 0.5),
                "work_ms_p95": percentile(&work, 0.95),
                "work_ms_max": work.last().copied().unwrap_or(0.),
            });
            (id.to_string(), json!({ "summary": summary, "frames": frames }))
        })
        .collect();
    json!({ "windows": windows })
}

/// Writes the snapshot's files into `dir`: `log.ndjson`, `tree.json`, `input.ndjson`,
/// `frames/layout.ndjson`, `profile.json`, `meta.json`.
pub fn write_snapshot(dir: &Path, snapshot: &Snapshot) -> io::Result<()> {
    write_lines(&dir.join("log.ndjson"), &snapshot.log)?;
    write_json(&dir.join("tree.json"), &json!({ "windows": snapshot.trees, "native": snapshot.native }))?;
    write_lines(&dir.join("input.ndjson"), &snapshot.inputs)?;
    fs::create_dir_all(dir.join("frames"))?;
    write_lines(&dir.join("frames/layout.ndjson"), &snapshot.layout)?;
    write_json(&dir.join("profile.json"), &profile(&snapshot.frames))?;
    write_json(&dir.join("meta.json"), &snapshot.meta)
}

/// Bundle caps (ARCHITECTURE.md 9.6).
#[derive(Clone, Copy, Debug)]
pub struct Retention {
    pub max_bundles: usize,
    pub max_age: Duration,
    pub max_bytes: u64,
}

impl Default for Retention {
    fn default() -> Self {
        Self { max_bundles: 20, max_age: Duration::from_secs(14 * 24 * 3600), max_bytes: 200 * 1024 * 1024 }
    }
}

fn dir_size(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            _ => e.metadata().map_or(0, |m| m.len()),
        })
        .sum()
}

fn is_bundle(name: &str) -> bool {
    name.len() > 24 && name.as_bytes()[..8].iter().all(u8::is_ascii_digit) && name.as_bytes()[8] == b'T'
}

/// A bundle on disk: `id` is the directory name, `<utc timestamp>-<kind>`.
#[derive(Clone, Debug)]
pub struct BundleInfo {
    pub id: String,
    pub path: PathBuf,
    pub kind: String,
    /// Time since the bundle was written, from its name. `None` for a name that does not parse.
    pub age: Option<Duration>,
}

/// The bundles in `dir`, newest first. A missing directory has none.
pub fn list_bundles(dir: &Path) -> io::Result<Vec<BundleInfo>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let now = chrono::Utc::now();
    let mut found: Vec<BundleInfo> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
        .filter(|(name, _)| is_bundle(name))
        .map(|(id, path)| {
            let (stamp, kind) = id.split_once('-').unwrap_or((&id, ""));
            let age = chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M%S%.fZ")
                .ok()
                .and_then(|t| (now - t.and_utc()).to_std().ok());
            BundleInfo { kind: kind.to_owned(), age, id, path }
        })
        .collect();
    found.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(found)
}

/// Deletes bundles beyond the newest `max_bundles`, older than `max_age`, and the oldest while
/// the total exceeds `max_bytes`. Returns what it deleted.
pub fn enforce_retention(dir: &Path, policy: Retention) -> io::Result<Vec<PathBuf>> {
    let mut bundles: Vec<(String, PathBuf)> = match fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
            .filter(|(name, _)| is_bundle(name))
            .collect(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    // Newest first.
    bundles.sort_by(|a, b| b.0.cmp(&a.0));
    let now = SystemTime::now();
    let mut keep = Vec::new();
    let mut doomed = Vec::new();
    for (i, (_, path)) in bundles.into_iter().enumerate() {
        let age = fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| now.duration_since(t).ok());
        if i >= policy.max_bundles || age.is_some_and(|a| a > policy.max_age) {
            doomed.push(path);
        } else {
            keep.push((dir_size(&path), path));
        }
    }
    let mut total: u64 = keep.iter().map(|(size, _)| size).sum();
    while total > policy.max_bytes
        && let Some((size, path)) = keep.pop()
    {
        total -= size;
        doomed.push(path);
    }
    for path in &doomed {
        fs::remove_dir_all(path)?;
    }
    Ok(doomed)
}

/// Deletes rotated log files older than `max_age`; the live log is never touched.
pub fn clean_logs(dir: &Path, live: &str, max_age: Duration) -> io::Result<()> {
    let now = SystemTime::now();
    for entry in fs::read_dir(dir)?.flatten() {
        if entry.file_name() == live {
            continue;
        }
        let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| now.duration_since(t).ok());
        if old.is_some_and(|a| a > max_age) {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}
