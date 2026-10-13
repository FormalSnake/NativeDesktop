//! Stand-in for `nd-theme` (M3): the token names apps read, with fixed light values. On macOS
//! the real tokens resolve from `NSColor` semantic colours.

use std::sync::LazyLock;

use nd_ui::App;
use nd_ui::prelude::{Hsla, hsla};

#[derive(Clone, Debug)]
pub struct TextTokens {
    pub primary: Hsla,
    pub secondary: Hsla,
    pub tertiary: Hsla,
    pub on_accent: Hsla,
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub text: TextTokens,
    pub accent: Hsla,
}

static DEFAULT: LazyLock<Theme> = LazyLock::new(|| Theme {
    text: TextTokens {
        primary: hsla(0., 0., 0., 0.85),
        secondary: hsla(0., 0., 0., 0.5),
        tertiary: hsla(0., 0., 0., 0.25),
        on_accent: hsla(0., 0., 1., 1.),
    },
    accent: hsla(211. / 360., 1., 0.5, 1.),
});

impl Theme {
    /// The macOS preset: tokens read from `NSColor` semantic colours at launch.
    #[cfg(target_os = "macos")]
    pub(crate) fn from_system(c: nd_backend_appkit::SystemColors) -> Self {
        Theme {
            text: TextTokens {
                primary: c.label,
                secondary: c.secondary_label,
                tertiary: c.tertiary_label,
                on_accent: hsla(0., 0., 1., 1.),
            },
            accent: c.accent,
        }
    }
}

pub trait ActiveTheme {
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Theme {
        self.try_global::<Theme>().unwrap_or(&DEFAULT)
    }
}
