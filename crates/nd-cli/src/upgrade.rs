//! `nd upgrade [--to <tag>]`: moves an app's nd dependency and its GPUI patch block to another
//! release tag, re-resolves the lockfile and prints the CHANGELOG entries in between.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::Args;

#[derive(Args, Debug)]
pub struct UpgradeArgs {
    /// The tag to move to. Defaults to the newest `vX.Y.Z` tag on the remote.
    #[arg(long)]
    pub to: Option<String>,
    /// The app's directory.
    #[arg(long, default_value = ".")]
    pub path: PathBuf,
}

pub fn run(args: UpgradeArgs) -> Result<()> {
    let manifest_path = args.path.join("Cargo.toml");
    let manifest =
        std::fs::read_to_string(&manifest_path).with_context(|| format!("reading {}", manifest_path.display()))?;
    let (url, from) = nd_source(&manifest).with_context(|| {
        format!("{} has no `nd = {{ git = \"...\", tag = \"...\" }}` dependency", manifest_path.display())
    })?;
    let to = match args.to {
        Some(tag) => tag,
        None => latest_tag(&url)?,
    };
    if to == from {
        println!("nd is already at {to}");
        return Ok(());
    }
    let upgraded = retag(&manifest, &url, &from, &to);
    std::fs::write(&manifest_path, &upgraded).with_context(|| format!("writing {}", manifest_path.display()))?;
    let checkout = match resolve(&manifest_path) {
        Ok(checkout) => checkout,
        Err(error) => {
            std::fs::write(&manifest_path, &manifest)
                .with_context(|| format!("restoring {}", manifest_path.display()))?;
            return Err(error.context(format!("resolving nd {to}; Cargo.toml is back at {from}")));
        }
    };
    println!("nd {from} -> {to}");
    let changelog = checkout.ancestors().map(|dir| dir.join("CHANGELOG.md")).find(|path| path.is_file());
    let entries = match changelog {
        Some(path) => {
            let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            changelog_between(&text, &from, &to)
        }
        None => String::new(),
    };
    if entries.is_empty() {
        println!("no CHANGELOG entries between {from} and {to}");
    } else {
        println!("\n{entries}");
    }
    Ok(())
}

/// The git URL and tag of the `nd` dependency.
fn nd_source(manifest: &str) -> Option<(String, String)> {
    let line = manifest.lines().find(|line| line.trim_start().starts_with("nd = {") && line.contains("git = "))?;
    Some((quoted_after(line, "git = ")?, quoted_after(line, "tag = ")?))
}

fn quoted_after(line: &str, key: &str) -> Option<String> {
    let rest = line.split_once(key)?.1.strip_prefix('"')?;
    Some(rest.split_once('"')?.0.to_string())
}

/// `manifest` with every dependency and patch on `url` at tag `from` moved to `to`.
fn retag(manifest: &str, url: &str, from: &str, to: &str) -> String {
    let git = format!("git = \"{url}\"");
    let (old, new) = (format!("tag = \"{from}\""), format!("tag = \"{to}\""));
    let mut out: String = manifest
        .lines()
        .map(|line| if line.contains(&git) { line.replace(&old, &new) } else { line.to_string() })
        .collect::<Vec<_>>()
        .join("\n");
    if manifest.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Re-resolves the lockfile for the edited manifest, keeping every other locked version, and
/// returns the directory cargo checked nd out to.
fn resolve(manifest_path: &Path) -> Result<PathBuf> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(&cargo)
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(manifest_path)
        .output()
        .context("running cargo metadata")?;
    if !output.status.success() {
        bail!("cargo metadata failed:\n{}", String::from_utf8_lossy(&output.stderr));
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).context("parsing cargo metadata")?;
    metadata["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|package| {
            package["name"] == "nd" && package["source"].as_str().is_some_and(|source| source.starts_with("git+"))
        })
        .and_then(|package| package["manifest_path"].as_str())
        .and_then(|path| Path::new(path).parent().map(Path::to_path_buf))
        .context("cargo metadata lists no nd package from git")
}

/// The newest `vX.Y.Z` tag on `url`.
fn latest_tag(url: &str) -> Result<String> {
    let output =
        Command::new("git").args(["ls-remote", "--tags", "--refs", url]).output().context("running git ls-remote")?;
    if !output.status.success() {
        bail!("git ls-remote {url} failed:\n{}", String::from_utf8_lossy(&output.stderr));
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_once("refs/tags/").map(|(_, tag)| tag.trim().to_string()))
        .filter_map(|tag| version(&tag).map(|v| (v, tag)))
        .max()
        .map(|(_, tag)| tag)
        .with_context(|| format!("{url} has no vX.Y.Z tag"))
}

fn version(tag: &str) -> Option<(u64, u64, u64)> {
    let mut parts = tag.strip_prefix('v').unwrap_or(tag).split('.').map(|part| part.parse().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// The `## vX.Y.Z` sections of `changelog` newer than `from` and no newer than `to`.
fn changelog_between(changelog: &str, from: &str, to: &str) -> String {
    let (Some(from), Some(to)) = (version(from), version(to)) else { return String::new() };
    let mut out = Vec::new();
    let mut keep = false;
    for line in changelog.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            keep = heading.split_whitespace().next().and_then(version).is_some_and(|v| v > from && v <= to);
        }
        if keep {
            out.push(line);
        }
    }
    out.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = "[dependencies]\nnd = { git = \"https://x/nd\", tag = \"v0.6.0\" }\n\n\
        [dev-dependencies]\nnd = { git = \"https://x/nd\", tag = \"v0.6.0\", features = [\"test\"] }\n\
        other = { git = \"https://y/other\", tag = \"v0.6.0\" }\n\n\
        [patch.crates-io]\ngpui-pre = { git = \"https://x/nd\", tag = \"v0.6.0\" }\n";

    #[test]
    fn retag_moves_nd_and_its_patches_and_nothing_else() {
        assert_eq!(nd_source(MANIFEST), Some(("https://x/nd".into(), "v0.6.0".into())));
        let out = retag(MANIFEST, "https://x/nd", "v0.6.0", "v0.7.1");
        assert_eq!(out.matches("tag = \"v0.7.1\"").count(), 3, "{out}");
        assert!(out.contains("other = { git = \"https://y/other\", tag = \"v0.6.0\" }"), "{out}");
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn versions_order_numerically() {
        assert!(version("v0.10.0") > version("v0.9.9"));
        assert_eq!(version("v1.2"), None);
        assert_eq!(version("v1.2.3-rc1"), None);
    }

    #[test]
    fn changelog_entries_are_the_ones_after_from_up_to_to() {
        let log = "# Changelog\n\n## v0.8.0\n\n- eight\n\n## v0.7.0 (2026-11-01)\n\n- seven\n\n## v0.6.0\n\n- six\n";
        let out = changelog_between(log, "v0.6.0", "v0.7.0");
        assert_eq!(out, "## v0.7.0 (2026-11-01)\n\n- seven");
        assert!(changelog_between(log, "v0.6.0", "v0.8.0").contains("- eight\n\n## v0.7.0"));
        assert_eq!(changelog_between(log, "v0.8.0", "v0.6.0"), "");
    }
}
