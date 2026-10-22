//! matugen `theme.json` (ARCHITECTURE.md 4, `docs/research/linux-nix-config.md` section 1).
//! The app never runs matugen; the desktop's matugen run renders nd's template
//! (`templates/matugen/theme.json`) to `~/.config/<app>/theme.json`. Three shapes parse:
//!
//! - per role, nd's template: `{"colors": {"primary": {"light": "#..", "dark": "#.."}}}`
//!   (`default` is accepted for either scheme);
//! - flat: `{"mode": "dark", "colors": {"primary": "#rrggbb", ...}}`, one scheme, or both when
//!   `mode` is absent;
//! - per scheme, as `matugen --json hex` dumps it: `{"colors": {"light": {"primary": "#.."}}}`;
//! - FormalShell's `~/.local/state/formalshell/theme.json`: shadcn keys at the top level next to
//!   `mode` (`{"mode": "dark", "background": "#..", "primary": "#.."}`), mapped by [`SHADCN_MAP`].
//!
//! Unknown roles and values that are not hex colours are ignored; a file that is not JSON is
//! an error, so the caller keeps the last good palette.

use serde_json::Value;

use crate::color::parse_hex;
use crate::palette::{PaletteRefinement, Scheme, SchemePalettes};

/// Material You role to nd token. `green` and `yellow` are matugen `custom_colors`, since
/// Material has no success or warning role. Sidebars sit on `surface_container` beside cards,
/// as the GTK colours the same desktop renders place them (FormalShell's
/// `gtk-colors.css.tmpl`), so a sidebar matches the GNOME apps next to it.
pub const ROLE_MAP: &[(&str, &str)] = &[
    ("surface", "surface.base"),
    ("surface_container", "surface.sidebar"),
    ("surface_container", "surface.raised"),
    ("surface_container_high", "surface.overlay"),
    ("outline_variant", "separator"),
    ("on_surface", "text.primary"),
    ("on_surface_variant", "text.secondary"),
    ("outline", "text.tertiary"),
    ("primary", "accent"),
    ("on_primary", "text.on_accent"),
    ("primary_container", "accent.container"),
    ("on_primary_container", "accent.on_container"),
    ("error", "danger"),
    ("secondary_container", "selection"),
    ("green", "success"),
    ("yellow", "warning"),
];

/// FormalShell's shadcn key to nd token. Only keys that carry the same Material role are
/// mapped: its `warning` is the tertiary colour and `accent` a hover fill, so neither is read.
/// `secondary` and `popover` are `surface_container`, placed as [`ROLE_MAP`] places it.
pub const SHADCN_MAP: &[(&str, &str)] = &[
    ("background", "surface.base"),
    ("secondary", "surface.sidebar"),
    ("popover", "surface.raised"),
    ("secondary", "surface.overlay"),
    ("border", "separator"),
    ("foreground", "text.primary"),
    ("mutedForeground", "text.secondary"),
    ("primary", "accent"),
    ("primaryForeground", "text.on_accent"),
    ("destructive", "danger"),
];

fn tokens_for(role: &str) -> impl Iterator<Item = &'static str> {
    ROLE_MAP.iter().filter(move |(r, _)| *r == role).map(|(_, t)| *t)
}

/// A parsed `theme.json`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MatugenTheme {
    /// The scheme the desktop is in, when the file says.
    pub mode: Option<Scheme>,
    pub palettes: SchemePalettes,
}

impl MatugenTheme {
    /// Whether the file set any colour nd knows.
    pub fn has_colours(&self) -> bool {
        self.palettes != SchemePalettes::default()
    }
}

#[derive(Debug)]
pub enum ParseError {
    Json(serde_json::Error),
    /// Valid JSON without a `colors` object.
    NoColors,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Json(e) => write!(f, "theme.json is not valid JSON: {e}"),
            ParseError::NoColors => write!(f, "theme.json has no \"colors\" object"),
        }
    }
}

impl std::error::Error for ParseError {}

fn scheme_named(name: &str) -> Option<Scheme> {
    match name {
        "light" => Some(Scheme::Light),
        "dark" | "amoled" => Some(Scheme::Dark),
        _ => None,
    }
}

/// A colour value: `"#rrggbb"` or `{"color": "#.."}` / `{"hex": "#.."}`.
fn color(value: &Value) -> Option<nd_gpui::gpui::Hsla> {
    match value {
        Value::String(s) => parse_hex(s),
        Value::Object(o) => o.get("color").or_else(|| o.get("hex")).and_then(Value::as_str).and_then(parse_hex),
        _ => None,
    }
}

fn set(palette: &mut PaletteRefinement, role: &str, value: &Value) {
    let Some(color) = color(value) else { return };
    for token in tokens_for(role) {
        palette.set(token, color);
    }
}

impl MatugenTheme {
    pub fn parse(text: &str) -> Result<MatugenTheme, ParseError> {
        let root: Value = serde_json::from_str(text).map_err(ParseError::Json)?;
        let mode = root.get("mode").and_then(Value::as_str).and_then(scheme_named);
        let Some(colors) = root.get("colors").and_then(Value::as_object) else {
            return Self::parse_shadcn(&root, mode);
        };
        let mut theme = MatugenTheme { mode, palettes: SchemePalettes::default() };

        let per_scheme = colors.keys().any(|k| scheme_named(k).is_some())
            && colors.iter().all(|(k, v)| scheme_named(k).is_none() || v.is_object());
        if per_scheme {
            for (name, roles) in colors {
                let (Some(scheme), Some(roles)) = (scheme_named(name), roles.as_object()) else { continue };
                // `amoled` is a darker dark; `dark` wins when both are present.
                if name == "amoled" && colors.contains_key("dark") {
                    continue;
                }
                for (role, value) in roles {
                    set(theme.palettes.get_mut(scheme), role, value);
                }
            }
            return Ok(theme);
        }

        for (role, value) in colors {
            match value {
                Value::Object(o) if o.keys().any(|k| matches!(k.as_str(), "light" | "dark" | "default")) => {
                    for scheme in [Scheme::Light, Scheme::Dark] {
                        let v = o.get(scheme.name()).or_else(|| o.get("default"));
                        if let Some(v) = v {
                            set(theme.palettes.get_mut(scheme), role, v);
                        }
                    }
                }
                flat => {
                    // A flat file renders `.default`, the desktop's current scheme; with no
                    // `mode` the colours stand for both.
                    match mode {
                        Some(scheme) => set(theme.palettes.get_mut(scheme), role, flat),
                        None => {
                            set(&mut theme.palettes.light, role, flat);
                            set(&mut theme.palettes.dark, role, flat);
                        }
                    }
                }
            }
        }
        Ok(theme)
    }

    fn parse_shadcn(root: &Value, mode: Option<Scheme>) -> Result<MatugenTheme, ParseError> {
        let keys = root.as_object().ok_or(ParseError::NoColors)?;
        if !keys.get("background").is_some_and(Value::is_string) {
            return Err(ParseError::NoColors);
        }
        let mut theme = MatugenTheme { mode, palettes: SchemePalettes::default() };
        for (key, token) in SHADCN_MAP {
            let Some(color) = keys.get(*key).and_then(color) else { continue };
            match mode {
                Some(scheme) => {
                    theme.palettes.get_mut(scheme).set(token, color);
                }
                None => {
                    theme.palettes.light.set(token, color);
                    theme.palettes.dark.set(token, color);
                }
            }
        }
        Ok(theme)
    }
}

/// nd's matugen template, for apps and the home-manager module to install.
pub const TEMPLATE: &str = include_str!("../templates/matugen/theme.json");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nd_template_shape() {
        let t = MatugenTheme::parse(r##"{"mode":"dark","colors":{"primary":"#aabbcc","surface":"#101010","nope":"#ffffff","green":"not a colour"}}"##).unwrap();
        assert_eq!(t.mode, Some(Scheme::Dark));
        assert_eq!(t.palettes.dark.accent, parse_hex("#aabbcc"));
        assert_eq!(t.palettes.dark.surface, parse_hex("#101010"));
        assert!(t.palettes.light.is_empty());
        let t = MatugenTheme::parse(r##"{"mode":"dark","colors":{"surface_container":"#282726"}}"##).unwrap();
        assert_eq!(t.palettes.dark.surface_sidebar, parse_hex("#282726"), "sidebars on surface_container, as GTK's");
        assert_eq!(t.palettes.dark.surface_raised, parse_hex("#282726"));
        assert!(t.palettes.dark.success.is_none());
    }

    #[test]
    fn per_role_shape() {
        let t = MatugenTheme::parse(
            r##"{"colors":{"primary":{"light":"#111111","dark":"#eeeeee"},"error":{"default":"#ff0000"}}}"##,
        )
        .unwrap();
        assert_eq!(t.palettes.light.accent, parse_hex("#111111"));
        assert_eq!(t.palettes.dark.accent, parse_hex("#eeeeee"));
        assert_eq!(t.palettes.dark.danger, parse_hex("#ff0000"));
    }

    #[test]
    fn per_scheme_shape() {
        let t = MatugenTheme::parse(
            r##"{"colors":{"light":{"primary":"#111111"},"dark":{"primary":"#eeeeee"},"amoled":{"primary":"#000000"}}}"##,
        )
        .unwrap();
        assert_eq!(t.palettes.light.accent, parse_hex("#111111"));
        assert_eq!(t.palettes.dark.accent, parse_hex("#eeeeee"));
    }

    #[test]
    fn formalshell_shape() {
        let t = MatugenTheme::parse(
            r##"{"mode":"dark","background":"#111318","foreground":"#e2e2e9","card":"#1a1b20","popover":"#1e1f25",
            "primary":"#adc6ff","primaryForeground":"#102f60","secondary":"#282a2f","mutedForeground":"#c4c6d0",
            "accent":"#33353a","destructive":"#ffb4ab","warning":"#debcdf","border":"#44474f","chart1":"#adc6ff"}"##,
        )
        .unwrap();
        assert_eq!(t.mode, Some(Scheme::Dark));
        assert_eq!(t.palettes.dark.surface, parse_hex("#111318"));
        // As the desktop's GTK colours place them: the sidebar on `secondary`.
        assert_eq!(t.palettes.dark.surface_sidebar, parse_hex("#282a2f"));
        assert_eq!(t.palettes.dark.surface_raised, parse_hex("#1e1f25"));
        assert_eq!(t.palettes.dark.accent, parse_hex("#adc6ff"));
        assert_eq!(t.palettes.dark.danger, parse_hex("#ffb4ab"));
        assert!(t.palettes.dark.warning.is_none());
        assert!(t.palettes.light.is_empty());
    }

    #[test]
    fn half_written_file_is_an_error() {
        assert!(MatugenTheme::parse(r##"{"colors":{"primary":"#11"##).is_err());
        assert!(matches!(MatugenTheme::parse("{}"), Err(ParseError::NoColors)));
    }

    #[test]
    fn template_maps_every_role() {
        for (role, _) in ROLE_MAP {
            assert!(TEMPLATE.contains(&format!("\"{role}\"")), "template is missing {role}");
        }
    }
}
