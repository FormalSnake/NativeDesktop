//! `nd check`: fmt, clippy with the framework lint table, then the syntactic framework lints.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};
use clap::Args;

use crate::lints;

#[derive(Args, Debug)]
pub struct CheckArgs {
    /// Check only these packages. Defaults to the workspace's default members.
    #[arg(short, long = "package")]
    pub packages: Vec<String>,
    /// Also fail on `<!-- nd: ... -->` placeholders left in markdown files.
    #[arg(long)]
    pub release: bool,
    /// Skip clippy (fmt and the framework lints still run).
    #[arg(long)]
    pub no_clippy: bool,
    /// Skip the appearance audit (`tests/appearance.rs`).
    #[arg(long)]
    pub no_audit: bool,
    #[arg(long)]
    pub manifest_path: Option<PathBuf>,
}

/// The framework lint table, passed on the command line so it applies to every app, whether or
/// not its `Cargo.toml` carries `[lints]`. `allow-*-in-tests` come from `clippy.toml`.
const CLIPPY_DENY: &[&str] = &[
    "warnings",
    "clippy::unwrap_used",
    "clippy::expect_used",
    "clippy::dbg_macro",
    "clippy::todo",
    "clippy::print_stdout",
];

struct Package {
    name: String,
    dir: PathBuf,
}

fn packages(args: &CheckArgs) -> Result<Vec<Package>> {
    let mut cmd = Command::new("cargo");
    cmd.args(["metadata", "--no-deps", "--format-version", "1"]);
    if let Some(path) = &args.manifest_path {
        cmd.arg("--manifest-path").arg(path);
    }
    let output = cmd.output().context("running cargo metadata")?;
    if !output.status.success() {
        bail!("cargo metadata failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    let meta: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let default_ids: Vec<&str> =
        meta["workspace_default_members"].as_array().into_iter().flatten().filter_map(|v| v.as_str()).collect();
    let mut out = Vec::new();
    for pkg in meta["packages"].as_array().into_iter().flatten() {
        let name = pkg["name"].as_str().unwrap_or_default();
        let id = pkg["id"].as_str().unwrap_or_default();
        let selected =
            if args.packages.is_empty() { default_ids.contains(&id) } else { args.packages.iter().any(|p| p == name) };
        if selected && let Some(manifest) = pkg["manifest_path"].as_str() {
            let dir = Path::new(manifest).parent().map(Path::to_path_buf).unwrap_or_default();
            out.push(Package { name: name.to_string(), dir });
        }
    }
    for wanted in &args.packages {
        if !out.iter().any(|p| &p.name == wanted) {
            bail!("no workspace package named `{wanted}`");
        }
    }
    Ok(out)
}

fn step(label: &str, cmd: &mut Command) -> Result<bool> {
    println!("nd check: {label}");
    Ok(cmd.status().with_context(|| format!("running {label}"))?.success())
}

fn package_flags(args: &CheckArgs) -> Vec<String> {
    args.packages.iter().flat_map(|p| ["-p".to_string(), p.clone()]).collect()
}

pub fn run(args: CheckArgs) -> Result<ExitCode> {
    let packages = packages(&args)?;
    let mut ok = true;

    let mut fmt = Command::new("cargo");
    fmt.args(["fmt", "--check"]).args(package_flags(&args));
    if let Some(path) = &args.manifest_path {
        fmt.arg("--manifest-path").arg(path);
    }
    ok &= step("fmt", &mut fmt)?;

    println!("nd check: framework lints");
    let mut findings = 0;
    for package in &packages {
        for finding in lints::lint_dir(&package.dir, &package.dir.join("src"))? {
            // Paths print relative to the package, prefixed so a workspace run stays unambiguous.
            println!("{}/{finding}", package.name);
            findings += 1;
        }
    }
    if findings > 0 {
        println!("nd check: {findings} framework lint finding(s). `// nd-allow(rule): reason` silences one.");
        ok = false;
    }

    if args.release {
        println!("nd check: release placeholders");
        for package in &packages {
            for hit in placeholders(&package.dir)? {
                println!("{}/{hit}", package.name);
                ok = false;
            }
        }
    }

    if !args.no_clippy {
        let mut clippy = Command::new("cargo");
        clippy.args(["clippy", "--all-targets", "--no-deps"]).args(package_flags(&args));
        if let Some(path) = &args.manifest_path {
            clippy.arg("--manifest-path").arg(path);
        }
        clippy.arg("--");
        for lint in CLIPPY_DENY {
            clippy.args(["-D", lint]);
        }
        ok &= step("clippy", &mut clippy)?;
    }

    if !args.no_audit {
        for package in packages.iter().filter(|p| p.dir.join("tests/appearance.rs").is_file()) {
            let mut audit = Command::new("cargo");
            audit.args(["test", "-p", &package.name, "--test", "appearance"]);
            if let Some(path) = &args.manifest_path {
                audit.arg("--manifest-path").arg(path);
            }
            ok &= step(&format!("appearance audit ({})", package.name), &mut audit)?;
        }
    }

    Ok(if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

/// `file:line: ..` for each `<!-- nd: ... -->` in the package's markdown files.
fn placeholders(dir: &Path) -> Result<Vec<String>> {
    let mut hits = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "md") {
            let text = std::fs::read_to_string(&path)?;
            for (index, line) in text.lines().enumerate() {
                if line.contains("<!-- nd:") {
                    let shown = path.strip_prefix(dir).unwrap_or(&path);
                    hits.push(format!("{}:{}: placeholder left in release build", shown.display(), index + 1));
                }
            }
        }
    }
    Ok(hits)
}
