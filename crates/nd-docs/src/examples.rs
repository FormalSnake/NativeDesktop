//! `docs/examples.toml`: the example apps the docs embed, and how to run each one for a capture.
//!
//! ```toml
//! [[example]]
//! name = "hello"            # directory under examples/
//! package = "hello"         # cargo package (defaults to name)
//! title = "Hello"
//! summary = "The smallest app: a toolbar, a button and a count."
//! web = true                # builds for wasm32; the docs app runs it live
//! env = { HELLO_DEMO = "1" }
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Example {
    pub name: String,
    #[serde(default)]
    pub package: String,
    pub title: String,
    pub summary: String,
    /// Builds for `wasm32-unknown-unknown`; its embed's Web tab runs it.
    #[serde(default)]
    pub web: bool,
    /// Why the Web tab cannot run it (a native engine, a pty, a Swift leaf).
    #[serde(default)]
    pub web_reason: Option<String>,
    /// Cargo features the binary needs.
    #[serde(default)]
    pub features: Vec<String>,
    /// The binary when the package has several.
    #[serde(default)]
    pub bin: Option<String>,
    /// Environment for captures: demo mode, fixed state.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// No window to capture (a plugin or a command line demo); the embed shows only source.
    #[serde(default)]
    pub headless: bool,
    /// A short recording beside the stills, for examples whose point is motion.
    #[serde(default)]
    pub motion: Option<Motion>,
    /// Pixels the capture window is resized to, so every OS shows the same frame.
    #[serde(default)]
    pub size: Option<(u32, u32)>,
    /// The source hash, filled in when the site loads (over any value examples.toml gives). Read
    /// back from the docs app's build-time JSON.
    #[serde(default)]
    pub hash: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Motion {
    pub seconds: f32,
    /// Automation calls made while recording, one every `interval_ms`: `{ method, params }`.
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default = "default_interval")]
    pub interval_ms: u64,
}

fn default_interval() -> u64 {
    250
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    example: Vec<Example>,
    /// Directories under `examples/` that are deliberately not in the docs, with the reason.
    #[serde(default)]
    skip: BTreeMap<String, String>,
}

impl Example {
    pub fn dir(&self, repo: &Path) -> PathBuf {
        repo.join("examples").join(&self.name)
    }
}

/// The registry, with packages defaulted and hashes computed. Fails when an example directory
/// is neither registered nor skipped, so a new example cannot slip past the docs.
pub fn load(repo: &Path, framework_version: &str) -> Result<Vec<Example>> {
    let path = repo.join("docs/examples.toml");
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let file: File = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let mut examples = file.example;
    for example in &mut examples {
        if example.package.is_empty() {
            example.package = example.name.clone();
        }
        let dir = example.dir(repo);
        if !dir.join("Cargo.toml").is_file() {
            bail!("docs/examples.toml: `{}` has no examples/{}/Cargo.toml", example.name, example.name);
        }
        example.hash = source_hash(&dir, framework_version)?;
    }
    for entry in std::fs::read_dir(repo.join("examples"))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().join("Cargo.toml").is_file()
            && !examples.iter().any(|e| e.name == name)
            && !file.skip.contains_key(&name)
        {
            bail!("examples/{name} is not in docs/examples.toml; add it or list it under [skip] with a reason");
        }
    }
    Ok(examples)
}

/// FNV-1a over the example's sources (anchor lines left out, so marking a snippet does not
/// invalidate its captures) and the framework version. Captures carry the hash they were taken
/// at; a different hash means the screen may have changed.
pub fn source_hash(dir: &Path, framework_version: &str) -> Result<String> {
    let mut files = Vec::new();
    collect(dir, dir, &mut files)?;
    files.sort();
    let mut hash = Fnv::default();
    hash.write(framework_version.as_bytes());
    for relative in files {
        hash.write(relative.as_bytes());
        let bytes = std::fs::read(dir.join(&relative))?;
        match String::from_utf8(bytes) {
            Ok(text) => {
                for line in text.lines().filter(|l| !crate::snippets::is_anchor_line(l)) {
                    hash.write(line.as_bytes());
                    hash.write(b"\n");
                }
            }
            Err(e) => hash.write(e.as_bytes()),
        }
    }
    Ok(format!("{:016x}", hash.0))
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        // Tests and their goldens do not change what the app shows.
        if name.starts_with('.') || matches!(name, "target" | "tests" | ".build") {
            continue;
        }
        if path.is_dir() {
            collect(root, &path, out)?;
        } else if let Ok(relative) = path.strip_prefix(root) {
            out.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_do_not_change_the_hash() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "fn a() {}\n").unwrap();
        let before = source_hash(dir.path(), "0.1.0").unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "// docs:begin a\nfn a() {}\n// docs:end a\n").unwrap();
        assert_eq!(source_hash(dir.path(), "0.1.0").unwrap(), before);
        std::fs::write(dir.path().join("src/lib.rs"), "fn b() {}\n").unwrap();
        assert_ne!(source_hash(dir.path(), "0.1.0").unwrap(), before);
        assert_ne!(source_hash(dir.path(), "0.2.0").unwrap(), before);
    }
}
