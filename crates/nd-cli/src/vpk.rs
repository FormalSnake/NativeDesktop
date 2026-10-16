//! The Velopack CLI, pinned to the version of the `velopack` crate `nd-update` links, so packages,
//! deltas and feeds are always produced by the same Velopack the app runs.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub const VERSION: &str = "1.2.161";

/// `ND_VPK` when set, else the pinned `vpk` under the user cache, installed there with
/// `dotnet tool install` on first use.
pub fn locate() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("ND_VPK").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let dir = tool_dir();
    let exe = dir.join(if cfg!(windows) { "vpk.exe" } else { "vpk" });
    if exe.exists() {
        return Ok(exe);
    }
    let dotnet = which("dotnet").context(
        "packaging needs Velopack's `vpk`: install the .NET SDK (https://dot.net) so nd can fetch it, or set ND_VPK",
    )?;
    eprintln!("nd: installing vpk {VERSION} into {}", dir.display());
    let status = Command::new(dotnet)
        .args(["tool", "install", "vpk", "--version", VERSION, "--tool-path"])
        .arg(&dir)
        .status()
        .context("running dotnet tool install")?;
    if !status.success() || !exe.exists() {
        bail!("dotnet tool install vpk {VERSION} failed ({status})");
    }
    Ok(exe)
}

fn tool_dir() -> PathBuf {
    let home = || PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let cache = if cfg!(target_os = "macos") {
        home().join("Library/Caches")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default())
    } else {
        std::env::var_os("XDG_CACHE_HOME").map_or_else(|| home().join(".cache"), PathBuf::from)
    };
    cache.join("nd/tools").join(format!("vpk-{VERSION}"))
}

pub fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exe = format!("{program}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&path).map(|dir| dir.join(&exe)).find(|p| p.is_file())
}

/// A `vpk` invocation of `args`. An OS directive (`[linux]`, `[win]`) has to come before every
/// option. On macOS it runs with the system toolchain first on `PATH` and without Nix's
/// `SDKROOT`/`DEVELOPER_DIR`, because it shells out to `codesign`, `xcrun notarytool` and
/// `pkgbuild`.
pub fn command(vpk: &Path, args: &[String]) -> Command {
    let mut command = Command::new(vpk);
    let (directive, rest) = match args.split_first() {
        Some((first, rest)) if first.starts_with('[') => (Some(first), rest),
        _ => (None, args),
    };
    command.args(directive).args(["--skip-updates", "--legacyConsole"]).args(rest);
    if cfg!(target_os = "macos") {
        command.env_remove("SDKROOT").env_remove("DEVELOPER_DIR");
        let rest = std::env::var("PATH").unwrap_or_default();
        command.env("PATH", format!("/usr/bin:/bin:/usr/sbin:/sbin:{rest}"));
    }
    command
}

/// Runs `args` through `vpk`, failing with its output.
pub fn run(vpk: &Path, args: &[String]) -> Result<()> {
    let output = command(vpk, args).output().with_context(|| format!("running {}", vpk.display()))?;
    if !output.status.success() {
        bail!(
            "vpk {} failed ({})\n{}{}",
            args.first().map_or("", String::as_str),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}
