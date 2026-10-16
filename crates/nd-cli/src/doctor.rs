//! `nd doctor`: whether what the app in the current project needs at run time is in place. Today
//! that is its web engine: Chromium's runtime beside the Linux executable (and the system
//! libraries `libcef.so` links), or the CEF distribution a macOS package bundles.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::Result;
use clap::Args;

use crate::package::cef;
use crate::project::Project;

#[derive(Args, Debug)]
pub struct DoctorArgs {
    /// The app's directory, when not the current one.
    #[arg(long, default_value = ".")]
    pub path: PathBuf,
}

pub fn run(args: DoctorArgs) -> Result<ExitCode> {
    let project = Project::load(&args.path)?;
    let problems = web_engine(&project);
    if problems.is_empty() {
        println!("nd doctor: {} is ready", project.name);
        return Ok(ExitCode::SUCCESS);
    }
    for problem in &problems {
        println!("nd doctor: {problem}");
    }
    Ok(ExitCode::FAILURE)
}

fn web_engine(project: &Project) -> Vec<String> {
    if cfg!(target_os = "linux") {
        return linux_web_engine(project);
    }
    if cef::enabled(project)
        && let Err(e) = cef::distribution(project)
    {
        return vec![format!("web views: the Chromium engine is missing: {e:#}")];
    }
    Vec::new()
}

/// Every build of the app that links Chromium, checked for its runtime and libraries.
fn linux_web_engine(project: &Project) -> Vec<String> {
    let mut problems = Vec::new();
    for profile in ["debug", "release"] {
        let exe = project.target_dir.join(profile).join(&project.bin);
        if !exe.exists() || !cef::links_cef(&exe).unwrap_or(false) {
            continue;
        }
        let dir = exe.parent().unwrap_or(Path::new("."));
        let missing = cef::linux_runtime_missing(dir);
        if !missing.is_empty() {
            problems.push(format!(
                "web views: the Chromium runtime beside {} is incomplete ({} missing): rebuild, or copy the \
                 CEF distribution there",
                exe.display(),
                missing.join(", ")
            ));
            continue;
        }
        for lib in unresolved(&dir.join("libcef.so")) {
            problems.push(format!(
                "web views: libcef.so needs {lib}, which the system does not have (install nss, gtk3, \
                 at-spi2, cups, libdrm, mesa's libgbm and alsa-lib, or run from `nix develop`)"
            ));
        }
    }
    problems
}

/// Libraries `ldd` cannot find for `lib`.
fn unresolved(lib: &Path) -> Vec<String> {
    let Ok(out) = Command::new("ldd").arg(lib).output() else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains("not found"))
        .filter_map(|l| l.split_whitespace().next().map(str::to_owned))
        .collect()
}
