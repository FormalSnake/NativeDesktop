//! The docs as `build.rs` resolved them: every page, the example registry, and per example and
//! OS the capture state from `assets/captures/manifest.toml`.

use std::collections::BTreeMap;

use nd_docs::Site;
use nd_docs::captures::{Os, State};
use serde::Deserialize;

const JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/content.json"));

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Content {
    pub site: Site,
    /// `example name -> os -> state`.
    pub captures: BTreeMap<String, BTreeMap<Os, State>>,
}

impl Content {
    pub fn load() -> Result<Content, String> {
        serde_json::from_str(JSON).map_err(|e| e.to_string())
    }

    pub fn capture(&self, example: &str, os: Os) -> &State {
        self.captures.get(example).and_then(|m| m.get(&os)).unwrap_or(&State::Missing)
    }
}
