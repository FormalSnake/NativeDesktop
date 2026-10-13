//! `theme.toml`: the app's or the user's palette, first in the chain. Token names as keys,
//! hex strings as values; top-level keys apply to both schemes, `[light]` and `[dark]` to one.
//!
//! ```toml
//! appearance = "dark"        # system | light | dark
//! radius_scale = 1.25
//! accent = "#3d6bff"
//! [dark]
//! "surface.base" = "#101014"
//! ```

use crate::color::parse_hex;
use crate::palette::{PaletteRefinement, Scheme};
use crate::refinement::{Appearance, ThemeRefinement};

#[derive(Debug)]
pub enum TomlError {
    Syntax(toml::de::Error),
    /// A key or value nd does not know. The whole file is rejected so a typo is noticed.
    Invalid(String),
}

impl std::fmt::Display for TomlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TomlError::Syntax(e) => write!(f, "theme.toml: {e}"),
            TomlError::Invalid(what) => write!(f, "theme.toml: {what}"),
        }
    }
}

impl std::error::Error for TomlError {}

fn colours(table: &toml::Table, into: &mut PaletteRefinement, section: &str) -> Result<(), TomlError> {
    for (key, value) in table {
        if value.is_table() || matches!(key.as_str(), "appearance" | "radius_scale") {
            continue;
        }
        let text = value.as_str().ok_or_else(|| TomlError::Invalid(format!("{section}{key} is not a string")))?;
        let color =
            parse_hex(text).ok_or_else(|| TomlError::Invalid(format!("{section}{key} = {text:?} is not hex")))?;
        if !into.set(key, color) {
            return Err(TomlError::Invalid(format!("unknown token {section}{key}")));
        }
    }
    Ok(())
}

pub fn parse(text: &str) -> Result<ThemeRefinement, TomlError> {
    let table: toml::Table = text.parse().map_err(TomlError::Syntax)?;
    let mut refinement = ThemeRefinement::new();
    if let Some(value) = table.get("appearance") {
        refinement.appearance = Some(match value.as_str() {
            Some("system") => Appearance::System,
            Some("light") => Appearance::Light,
            Some("dark") => Appearance::Dark,
            _ => return Err(TomlError::Invalid(format!("appearance = {value} is not system, light or dark"))),
        });
    }
    if let Some(value) = table.get("radius_scale") {
        let scale = value
            .as_float()
            .or_else(|| value.as_integer().map(|i| i as f64))
            .filter(|s| *s > 0.)
            .ok_or_else(|| TomlError::Invalid(format!("radius_scale = {value} is not a positive number")))?;
        refinement.radius_scale = Some(scale as f32);
    }
    let mut both = PaletteRefinement::default();
    colours(&table, &mut both, "")?;
    for scheme in [Scheme::Light, Scheme::Dark] {
        let mut own = PaletteRefinement::default();
        if let Some(section) = table.get(scheme.name()) {
            let section =
                section.as_table().ok_or_else(|| TomlError::Invalid(format!("[{}] is not a table", scheme.name())))?;
            colours(section, &mut own, &format!("{}.", scheme.name()))?;
        }
        let merged = own.over(both);
        match scheme {
            Scheme::Light => refinement.light = merged,
            Scheme::Dark => refinement.dark = merged,
        }
    }
    for key in table.keys() {
        let known = matches!(key.as_str(), "appearance" | "radius_scale" | "light" | "dark")
            || crate::palette::KEYS.contains(&key.as_str());
        if !known {
            return Err(TomlError::Invalid(format!("unknown key {key}")));
        }
    }
    Ok(refinement)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_override_top_level() {
        let r = parse(
            r##"
            appearance = "dark"
            radius_scale = 1.5
            accent = "#3d6bff"
            [dark]
            accent = "#ff8a3d"
            "surface.base" = "#101014"
            "##,
        )
        .unwrap();
        assert_eq!(r.appearance, Some(Appearance::Dark));
        assert_eq!(r.radius_scale, Some(1.5));
        assert_eq!(r.light.accent, parse_hex("#3d6bff"));
        assert_eq!(r.dark.accent, parse_hex("#ff8a3d"));
        assert_eq!(r.dark.surface, parse_hex("#101014"));
    }

    #[test]
    fn typos_are_rejected() {
        assert!(parse(r##"acent = "#ffffff""##).is_err());
        assert!(parse(r##"accent = "blue""##).is_err());
        assert!(parse(r##"appearance = "sepia""##).is_err());
    }
}
