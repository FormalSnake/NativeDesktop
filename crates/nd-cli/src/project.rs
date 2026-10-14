//! An app project: its Cargo package (name, version, binary, target dir) and `nativedesktop.toml`.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use nd_config::Config;

#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub name: String,
    pub version: String,
    /// The binary target `nd package` ships.
    pub bin: String,
    pub target_dir: PathBuf,
    pub config: Config,
}

impl Project {
    pub fn load(root: &Path) -> Result<Self> {
        let root = std::fs::canonicalize(root).with_context(|| format!("{}", root.display()))?;
        let manifest = root.join("Cargo.toml");
        let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
            .args(["metadata", "--format-version", "1", "--no-deps", "--manifest-path"])
            .arg(&manifest)
            .output()
            .context("running cargo metadata")?;
        if !output.status.success() {
            bail!("cargo metadata: {}", String::from_utf8_lossy(&output.stderr).trim());
        }
        let meta: serde_json::Value = serde_json::from_slice(&output.stdout).context("cargo metadata output")?;
        let package = meta["packages"]
            .as_array()
            .and_then(|ps| ps.iter().find(|p| p["manifest_path"].as_str().map(Path::new) == Some(manifest.as_path())))
            .with_context(|| format!("no package at {}", manifest.display()))?;
        let name = package["name"].as_str().context("package name")?.to_owned();
        let bins: Vec<&str> = package["targets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| t["kind"].as_array().is_some_and(|k| k.iter().any(|k| k == "bin")))
            .filter_map(|t| t["name"].as_str())
            .collect();
        let bin = if bins.contains(&name.as_str()) {
            name.clone()
        } else {
            bins.first().context("no binary target")?.to_string()
        };
        let config_path = root.join("nativedesktop.toml");
        let config = Config::load(&config_path)?;
        Ok(Self {
            name,
            version: package["version"].as_str().context("package version")?.to_owned(),
            bin,
            target_dir: PathBuf::from(meta["target_directory"].as_str().context("target_directory")?),
            config,
            root,
        })
    }

    pub fn display_name(&self) -> &str {
        self.config.display_name(&self.name)
    }

    pub fn id(&self) -> &str {
        &self.config.app.id
    }

    /// `target/nd`.
    pub fn nd_dir(&self) -> PathBuf {
        self.target_dir.join("nd")
    }

    /// The release build of the app binary.
    pub fn build_release(&self, target: Option<&str>) -> Result<PathBuf> {
        let mut cargo = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
        cargo.args(["build", "--release", "--bin", &self.bin]).current_dir(&self.root);
        if crate::package::cef::enabled(self) {
            cargo.args(["--features", "nd/cef"]);
        }
        if let Some(target) = target {
            cargo.args(["--target", target]);
        }
        let status = cargo.status().context("running cargo build")?;
        if !status.success() {
            bail!("cargo build --release failed ({status})");
        }
        let mut dir = self.target_dir.clone();
        if let Some(target) = target {
            dir.push(target);
        }
        let exe = dir.join("release").join(if target.is_some_and(|t| t.contains("windows")) || cfg!(windows) {
            format!("{}.exe", self.bin)
        } else {
            self.bin.clone()
        });
        if !exe.exists() {
            bail!("cargo build produced no {}", exe.display());
        }
        Ok(exe)
    }

    /// A path from `nativedesktop.toml`, relative to the project root.
    pub fn resolve(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// The release channel: `--channel`, else the first configured one. Must be configured when
    /// the app has `[updates]`.
    pub fn channel(&self, requested: Option<&str>) -> Result<String> {
        let channels = self.config.updates.channels();
        match requested {
            Some(c) if self.config.updates.feed.is_some() && !channels.iter().any(|x| x == c) => {
                bail!("channel `{c}` is not in updates.channels ({})", channels.join(", "))
            }
            Some(c) => Ok(c.to_owned()),
            None => Ok(channels[0].clone()),
        }
    }
}

/// Writes `text` to `path`, creating parent directories.
pub fn write(path: &Path, text: impl AsRef<[u8]>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

/// Removes and recreates a directory nd owns under `target/nd`.
pub fn fresh_dir(path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_dir_all(path).with_context(|| format!("clearing {}", path.display()))?;
    }
    std::fs::create_dir_all(path).with_context(|| format!("creating {}", path.display()))
}

pub fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
