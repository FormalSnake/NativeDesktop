//! Screenshots and recordings of the examples, one set per OS, taken by `nd docs capture` on that
//! OS through the test harness. They live in `docs/app/assets/captures/<example>/` with
//! `manifest.toml` beside them recording, per example and OS, the example's source hash at
//! capture time. A capture whose hash differs from the example's current one is stale and is
//! never shown: the embed says it is not captured yet instead.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::examples::Example;

pub const DIR: &str = "docs/app/assets/captures";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Macos,
    Linux,
    Windows,
}

impl Os {
    pub const ALL: [Os; 3] = [Os::Macos, Os::Linux, Os::Windows];

    pub fn key(self) -> &'static str {
        match self {
            Os::Macos => "macos",
            Os::Linux => "linux",
            Os::Windows => "windows",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Os::Macos => "macOS",
            Os::Linux => "Linux",
            Os::Windows => "Windows",
        }
    }

    pub fn current() -> Option<Os> {
        if cfg!(target_os = "macos") {
            Some(Os::Macos)
        } else if cfg!(target_os = "linux") {
            Some(Os::Linux)
        } else if cfg!(windows) {
            Some(Os::Windows)
        } else {
            None
        }
    }

    pub fn parse(text: &str) -> Option<Os> {
        Os::ALL.into_iter().find(|o| o.key() == text)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    /// The example's source hash when this was taken.
    pub hash: String,
    /// "macOS 27.0", with the machine where it matters.
    pub captured_on: String,
    pub date: String,
    /// Relative to the example's capture directory: `macos-light.png`, `macos-dark.png`.
    pub light: String,
    pub dark: String,
    /// A short GIF where motion matters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording: Option<String>,
}

/// `example name -> os -> capture`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Manifest(pub BTreeMap<String, BTreeMap<Os, Capture>>);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum State {
    Fresh(Capture),
    /// Taken for an older version of the example.
    Stale {
        captured: String,
    },
    Missing,
}

impl Manifest {
    pub fn path(repo: &Path) -> PathBuf {
        repo.join(DIR).join("manifest.toml")
    }

    pub fn load(repo: &Path) -> Result<Self> {
        let path = Self::path(repo);
        if !path.is_file() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self, repo: &Path) -> Result<()> {
        let path = Self::path(repo);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = toml::to_string_pretty(self)?;
        std::fs::write(&path, format!("# Written by `nd docs capture`; do not edit by hand.\n\n{body}"))
            .with_context(|| format!("writing {}", path.display()))
    }

    pub fn state(&self, example: &Example, os: Os) -> State {
        match self.0.get(&example.name).and_then(|m| m.get(&os)) {
            Some(c) if c.hash == example.hash => State::Fresh(c.clone()),
            Some(c) => State::Stale { captured: c.date.clone() },
            None => State::Missing,
        }
    }

    pub fn set(&mut self, example: &str, os: Os, capture: Capture) {
        self.0.entry(example.to_owned()).or_default().insert(os, capture);
    }
}

/// Per example and OS, the state the embed will show.
pub fn report(repo: &Path, examples: &[Example]) -> Result<Vec<(String, Os, State)>> {
    let manifest = Manifest::load(repo)?;
    let mut out = Vec::new();
    for example in examples.iter().filter(|e| !e.headless) {
        for os in Os::ALL {
            let state = manifest.state(example, os);
            if let State::Fresh(c) = &state {
                for file in [Some(&c.light), Some(&c.dark), c.recording.as_ref()].into_iter().flatten() {
                    let path = repo.join(DIR).join(&example.name).join(file);
                    if !path.is_file() {
                        anyhow::bail!("{}: manifest names {} but the file is missing", example.name, path.display());
                    }
                }
            }
            out.push((example.name.clone(), os, state));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_captures_are_not_fresh() {
        let mut manifest = Manifest::default();
        let example = Example { name: "hello".into(), hash: "aa".into(), ..Example::default() };
        assert_eq!(manifest.state(&example, Os::Macos), State::Missing);
        let capture = Capture { hash: "bb".into(), date: "2026-10-14".into(), ..Capture::default() };
        manifest.set("hello", Os::Macos, capture.clone());
        assert_eq!(manifest.state(&example, Os::Macos), State::Stale { captured: "2026-10-14".into() });
        manifest.set("hello", Os::Macos, Capture { hash: "aa".into(), ..capture });
        assert!(matches!(manifest.state(&example, Os::Macos), State::Fresh(_)));
        let text = toml::to_string_pretty(&manifest).unwrap();
        assert_eq!(toml::from_str::<Manifest>(&text).unwrap(), manifest);
    }
}
