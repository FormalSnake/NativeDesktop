//! The base colours every theme derives from: sixteen keys per scheme, named like the tokens
//! they feed (ARCHITECTURE.md 4). Sources fill them in order (`theme.toml`, matugen
//! `theme.json`, the OS, the baked-in palettes) and the first source to set a key wins.

use nd_gpui::gpui::Hsla;

use crate::color::parse_hex;

/// Light or dark, resolved. [`crate::Appearance`] is the preference that resolves to it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Scheme {
    #[default]
    Light,
    Dark,
}

impl Scheme {
    pub fn is_dark(self) -> bool {
        self == Scheme::Dark
    }

    pub fn name(self) -> &'static str {
        match self {
            Scheme::Light => "light",
            Scheme::Dark => "dark",
        }
    }
}

macro_rules! palette {
    ($($field:ident = $key:literal),* $(,)?) => {
        /// One scheme's base colours.
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct Palette {
            $(pub $field: Hsla,)*
        }

        /// A partial [`Palette`]: what one source sets.
        #[derive(Clone, Copy, Debug, Default, PartialEq)]
        pub struct PaletteRefinement {
            $(pub $field: Option<Hsla>,)*
        }

        /// The token name of every key, as `theme.toml` spells it.
        pub const KEYS: &[&str] = &[$($key),*];

        impl PaletteRefinement {
            /// Keys this refinement sets, by token name.
            pub fn keys(&self) -> Vec<&'static str> {
                let mut keys = Vec::new();
                $(if self.$field.is_some() { keys.push($key); })*
                keys
            }

            pub fn is_empty(&self) -> bool {
                $(self.$field.is_none())&&*
            }

            /// Sets `key` (a token name) to `color`. False for an unknown key.
            pub fn set(&mut self, key: &str, color: Hsla) -> bool {
                match key {
                    $($key => self.$field = Some(color),)*
                    _ => return false,
                }
                true
            }

            /// Keys set here win over `below`.
            pub fn over(self, below: PaletteRefinement) -> PaletteRefinement {
                PaletteRefinement { $($field: self.$field.or(below.$field),)* }
            }

            /// Fills every unset key from `base`.
            pub fn complete(self, base: &Palette) -> Palette {
                Palette { $($field: self.$field.unwrap_or(base.$field),)* }
            }
        }

        impl From<Palette> for PaletteRefinement {
            fn from(p: Palette) -> Self {
                PaletteRefinement { $($field: Some(p.$field),)* }
            }
        }
    };
}

palette! {
    surface = "surface.base",
    surface_sidebar = "surface.sidebar",
    surface_raised = "surface.raised",
    surface_overlay = "surface.overlay",
    text_primary = "text.primary",
    text_secondary = "text.secondary",
    text_tertiary = "text.tertiary",
    accent = "accent",
    on_accent = "text.on_accent",
    accent_container = "accent.container",
    on_accent_container = "accent.on_container",
    separator = "separator",
    selection = "selection",
    danger = "danger",
    success = "success",
    warning = "warning",
}

fn hex(text: &str) -> Hsla {
    parse_hex(text).expect("baked palettes use valid hex")
}

impl Palette {
    /// The baked-in palette of `preset`: Fluent's are WinUI 3's theme resources (Mica base
    /// under the sidebar, the layer fill under content, cards, the default accent, the system
    /// fill colours), so the Windows skin reads as Windows where the OS gives no colour and
    /// where it is drawn off Windows (`ND_PRESET=fluent`). Every other preset takes [`Palette::baked`].
    pub fn baked_for(preset: crate::Preset, scheme: Scheme) -> Palette {
        match (preset, scheme) {
            (crate::Preset::Fluent, Scheme::Light) => Palette {
                surface: hex("#f9f9f9"),
                surface_sidebar: hex("#f3f3f3"),
                surface_raised: hex("#ffffff"),
                surface_overlay: hex("#fcfcfc"),
                text_primary: hex("#000000e4"),
                text_secondary: hex("#0000009e"),
                text_tertiary: hex("#00000072"),
                accent: hex("#005fb8"),
                on_accent: hex("#ffffff"),
                accent_container: hex("#cce4f6"),
                on_accent_container: hex("#003e73"),
                separator: hex("#0000000f"),
                selection: hex("#cce4f6"),
                danger: hex("#c42b1c"),
                success: hex("#0f7b0f"),
                warning: hex("#9d5d00"),
            },
            (crate::Preset::Fluent, Scheme::Dark) => Palette {
                surface: hex("#272727"),
                surface_sidebar: hex("#202020"),
                surface_raised: hex("#2d2d2d"),
                surface_overlay: hex("#2c2c2c"),
                text_primary: hex("#ffffff"),
                text_secondary: hex("#ffffffc5"),
                text_tertiary: hex("#ffffff87"),
                accent: hex("#60cdff"),
                on_accent: hex("#000000"),
                accent_container: hex("#004a83"),
                on_accent_container: hex("#ffffff"),
                separator: hex("#ffffff15"),
                selection: hex("#004a83"),
                danger: hex("#ff99a4"),
                success: hex("#6ccb5f"),
                warning: hex("#fce100"),
            },
            // libadwaita's own named colours (`_colors.scss`): window, sidebar, card and popover
            // backgrounds, the window foreground and its 55% dimmed label, the default blue
            // accent, the selection's 25% of it, and the destructive, success and warning fills.
            (crate::Preset::Adwaita, Scheme::Light) => Palette {
                surface: hex("#fafafb"),
                surface_sidebar: hex("#ebebed"),
                surface_raised: hex("#ffffff"),
                surface_overlay: hex("#ffffff"),
                text_primary: hex("#000006cc"),
                text_secondary: hex("#00000670"),
                text_tertiary: hex("#0000064d"),
                accent: hex("#3584e4"),
                on_accent: hex("#ffffff"),
                accent_container: hex("#cce0f8"),
                on_accent_container: hex("#000006cc"),
                separator: hex("#00000626"),
                selection: hex("#3584e440"),
                danger: hex("#e01b24"),
                success: hex("#2ec27e"),
                warning: hex("#e5a50a"),
            },
            (crate::Preset::Adwaita, Scheme::Dark) => Palette {
                surface: hex("#222226"),
                surface_sidebar: hex("#2e2e32"),
                // A card is 8% white over the window.
                surface_raised: hex("#343438"),
                surface_overlay: hex("#36363a"),
                text_primary: hex("#ffffff"),
                text_secondary: hex("#ffffff8c"),
                text_tertiary: hex("#ffffff4d"),
                accent: hex("#3584e4"),
                on_accent: hex("#ffffff"),
                accent_container: hex("#233751"),
                on_accent_container: hex("#ffffff"),
                separator: hex("#ffffff26"),
                selection: hex("#3584e466"),
                danger: hex("#c01c28"),
                success: hex("#26a269"),
                warning: hex("#cd9309"),
            },
            _ => Palette::baked(scheme),
        }
    }

    /// The baked-in palette: the last source in the chain, used where nothing else answers.
    pub fn baked(scheme: Scheme) -> Palette {
        match scheme {
            Scheme::Light => Palette {
                surface: hex("#fafafa"),
                surface_sidebar: hex("#f0f0f2"),
                surface_raised: hex("#ffffff"),
                surface_overlay: hex("#ffffff"),
                text_primary: hex("#000000d9"),
                text_secondary: hex("#00000080"),
                text_tertiary: hex("#00000042"),
                accent: hex("#2f6fed"),
                on_accent: hex("#ffffff"),
                accent_container: hex("#d9e5ff"),
                on_accent_container: hex("#0b2a66"),
                separator: hex("#0000001a"),
                selection: hex("#cfdfff"),
                danger: hex("#d93025"),
                success: hex("#1e8e3e"),
                warning: hex("#c98a00"),
            },
            Scheme::Dark => Palette {
                surface: hex("#1e1e20"),
                surface_sidebar: hex("#252528"),
                surface_raised: hex("#2b2b2e"),
                surface_overlay: hex("#333336"),
                text_primary: hex("#ffffffd9"),
                text_secondary: hex("#ffffff8c"),
                text_tertiary: hex("#ffffff42"),
                accent: hex("#4c8dff"),
                // White on this accent is 3.2:1; the accent has to stay this light to read as
                // text on the dark surfaces, so what sits on it is dark, as Material's is.
                on_accent: hex("#0b1a33"),
                accent_container: hex("#1f3a6e"),
                on_accent_container: hex("#d9e5ff"),
                separator: hex("#ffffff1f"),
                selection: hex("#2c4a80"),
                danger: hex("#ff6b5e"),
                success: hex("#4cc46a"),
                warning: hex("#ffbf3c"),
            },
        }
    }
}

/// A source's palettes for both schemes. A key set for one scheme only applies to that one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SchemePalettes {
    pub light: PaletteRefinement,
    pub dark: PaletteRefinement,
}

impl SchemePalettes {
    pub fn get(&self, scheme: Scheme) -> &PaletteRefinement {
        match scheme {
            Scheme::Light => &self.light,
            Scheme::Dark => &self.dark,
        }
    }

    pub fn get_mut(&mut self, scheme: Scheme) -> &mut PaletteRefinement {
        match scheme {
            Scheme::Light => &mut self.light,
            Scheme::Dark => &mut self.dark,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.light.is_empty() && self.dark.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refinement_layers_first_wins() {
        let mut top = PaletteRefinement::default();
        top.set("accent", hex("#ff0000"));
        let mut below = PaletteRefinement::default();
        below.set("accent", hex("#00ff00"));
        below.set("danger", hex("#0000ff"));
        let merged = top.over(below).complete(&Palette::baked(Scheme::Light));
        assert_eq!(merged.accent, hex("#ff0000"));
        assert_eq!(merged.danger, hex("#0000ff"));
        assert_eq!(merged.surface, Palette::baked(Scheme::Light).surface);
    }

    #[test]
    fn every_key_is_settable() {
        let mut r = PaletteRefinement::default();
        for key in KEYS {
            assert!(r.set(key, hex("#123456")), "{key}");
        }
        assert_eq!(r.keys().len(), KEYS.len());
        assert!(!r.set("nope", hex("#123456")));
    }
}
