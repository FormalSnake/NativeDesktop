//! Design tokens, presets, palette sources and override layers (ARCHITECTURE.md 4).
//!
//! - [`Theme`]: the tokens apps read with `cx.theme()`, derived from one [`Palette`] and the
//!   platform [`Metrics`].
//! - [`ThemeRegistry`]: the source chain (`theme.toml`, matugen `theme.json`, OS, baked) under
//!   the app, window and subtree overrides, resolved per window.
//! - [`ThemeFileWatcher`]: live reload of `theme.json` and `theme.toml`.
//!
//! No backend code lives here: backends fill [`OsTheme`] from the OS and read [`NativeTheme`]
//! for native controls.

pub mod color;
pub mod fonts;
pub mod matugen;
pub mod palette;
pub mod preset;
pub mod refinement;
pub mod registry;
pub mod theme_toml;
pub mod tokens;
pub mod watcher;

use std::path::PathBuf;

pub use fonts::SystemFonts;
pub use matugen::MatugenTheme;
pub use palette::{Palette, PaletteRefinement, Scheme, SchemePalettes};
pub use preset::{
    ControlHeights, Fonts, GlassVariant, Material, Materials, Metrics, Motion, Preset, Radii, Spacing, Spring,
    SystemMetrics, TextStyle, TextStyles, TypeScale, TypeStyle,
};
pub use refinement::{Appearance, ThemeOverride, ThemeRefinement};
pub use registry::{NativeTheme, OsTheme, ThemeRegistry};
pub use tokens::{AccentContainer, BorderTokens, FontTokens, SurfaceTokens, TerminalTokens, TextTokens, Theme};
pub use watcher::ThemeFileWatcher;

/// Where an app's theme files live. `ND_THEME_JSON` / `ND_THEME_TOML` override each path (an
/// empty value turns the source off). matugen's `theme.json` is read by default on Linux
/// only; elsewhere it is opt-in through `ND_THEME_JSON` or [`ThemeFiles::matugen`]. On Linux
/// the first existing file wins: `<config>/<app>/theme.json`, then FormalShell's
/// `$XDG_STATE_HOME/formalshell/theme.json`; with neither present the app path is watched.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeFiles {
    pub toml: Option<PathBuf>,
    pub matugen: Option<PathBuf>,
}

fn config_dir(app: &str) -> PathBuf {
    let home = || PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    if cfg!(target_os = "macos") {
        home().join("Library/Application Support").join(app)
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join(app)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|d| !d.is_empty())
            .map_or_else(|| home().join(".config"), PathBuf::from)
            .join(app)
    }
}

/// The palette FormalShell publishes on every wallpaper change.
fn shell_theme_json() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME").filter(|d| !d.is_empty()).map(PathBuf::from).or_else(|| {
        std::env::var_os("HOME").filter(|d| !d.is_empty()).map(|h| PathBuf::from(h).join(".local/state"))
    })?;
    Some(state.join("formalshell/theme.json"))
}

fn discover(own: PathBuf, shell: Option<PathBuf>) -> PathBuf {
    if own.exists() {
        return own;
    }
    shell.filter(|p| p.exists()).unwrap_or(own)
}

fn from_env(name: &str) -> Option<Option<PathBuf>> {
    std::env::var_os(name).map(|v| (!v.is_empty()).then(|| PathBuf::from(v)))
}

impl ThemeFiles {
    /// The default locations for app `name`: `<config>/<name>/theme.toml` everywhere, and
    /// `~/.config/<name>/theme.json` on Linux, falling back to the shell's own palette file.
    pub fn for_app(name: &str) -> ThemeFiles {
        let dir = config_dir(name);
        let toml = from_env("ND_THEME_TOML").unwrap_or_else(|| Some(dir.join("theme.toml")));
        let matugen = from_env("ND_THEME_JSON").unwrap_or_else(|| {
            (cfg!(target_os = "linux") || cfg!(target_os = "freebsd"))
                .then(|| discover(dir.join("theme.json"), shell_theme_json()))
        });
        ThemeFiles { toml, matugen }
    }

    /// Reads matugen's file at `path` on any OS.
    pub fn matugen(mut self, path: impl Into<PathBuf>) -> Self {
        self.matugen = Some(path.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_prefers_the_app_file_then_the_shell_file() {
        let root = std::env::temp_dir().join(format!("nd-theme-discover-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let (own, shell) = (root.join("own.json"), root.join("shell.json"));
        assert_eq!(discover(own.clone(), Some(shell.clone())), own);
        std::fs::write(&shell, "{}").unwrap();
        assert_eq!(discover(own.clone(), Some(shell.clone())), shell);
        std::fs::write(&own, "{}").unwrap();
        assert_eq!(discover(own.clone(), Some(shell)), own);
        assert_eq!(discover(own.clone(), None), own);
    }
}
