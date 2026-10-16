//! The platform's UI fonts, read from the OS (ARCHITECTURE.md 3.10): the families every text
//! role uses and, where the OS publishes them, the text styles. Backends put the result in
//! [`crate::OsTheme::fonts`] at launch; [`crate::Metrics::with_fonts`] lays it over the preset.
//!
//! - macOS: the system font and its monospaced and rounded designs, as `NSFont` names them, and
//!   every `NSFont.TextStyle` with the line height `NSLayoutManager` gives it.
//! - Linux: the desktop's interface fonts (`org.gnome.desktop.interface` in dconf, GTK's
//!   `settings.ini`, then the GSettings default), else fontconfig's `sans-serif` and
//!   `monospace`. A family fontconfig cannot find is skipped.
//! - Windows: Segoe UI Variable and Cascadia Mono.
//! - The web has no system fonts: the preset's bundled ones stand.

use nd_gpui::gpui::SharedString;

use crate::preset::TextStyles;

/// What the OS names. Unset fields keep the preset's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SystemFonts {
    pub body: Option<SharedString>,
    /// Falls back to `body` where the OS has no rounded design.
    pub rounded: Option<SharedString>,
    pub mono: Option<SharedString>,
    pub text_styles: Option<TextStyles>,
}

#[cfg(target_os = "macos")]
pub use mac::resolve;

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub use desktop::resolve;

/// Segoe UI Variable's text optical size, the WinUI 3 body font.
#[cfg(windows)]
pub fn resolve() -> SystemFonts {
    SystemFonts {
        body: Some("Segoe UI Variable Text".into()),
        rounded: None,
        mono: Some("Cascadia Mono".into()),
        text_styles: None,
    }
}

/// Nothing: the preset's fonts stand.
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "freebsd", windows)))]
pub fn resolve() -> SystemFonts {
    SystemFonts::default()
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::rc::Retained;
    use objc2_app_kit::{
        NSFont, NSFontDescriptorSystemDesignRounded, NSFontTextStyle, NSFontTextStyleBody, NSFontTextStyleCallout,
        NSFontTextStyleCaption1, NSFontTextStyleCaption2, NSFontTextStyleFootnote, NSFontTextStyleHeadline,
        NSFontTextStyleLargeTitle, NSFontTextStyleSubheadline, NSFontTextStyleTitle1, NSFontTextStyleTitle2,
        NSFontTextStyleTitle3, NSFontWeightRegular,
    };
    use objc2_core_text::CTFont;
    use objc2_foundation::{NSDictionary, NSNumber};

    use super::SystemFonts;
    use crate::preset::{TextStyle, TextStyles};

    /// `'wght'`, the variation axis SF's weights sit on.
    const WEIGHT_AXIS: u32 = u32::from_be_bytes(*b"wght");

    pub fn resolve() -> SystemFonts {
        let family = |font: &NSFont| font.familyName().map(|f| f.to_string().into());
        let system = NSFont::systemFontOfSize(NSFont::systemFontSize());
        let mono = NSFont::monospacedSystemFontOfSize_weight(NSFont::systemFontSize(), unsafe { NSFontWeightRegular });
        let rounded = system
            .fontDescriptor()
            .fontDescriptorWithDesign(unsafe { NSFontDescriptorSystemDesignRounded })
            .and_then(|d| NSFont::fontWithDescriptor_size(&d, system.pointSize()));
        let rows = TextStyle::ALL.map(|style| {
            let font = unsafe { NSFont::preferredFontForTextStyle_options(text_style(style), &NSDictionary::new()) };
            // What `NSLayoutManager.defaultLineHeight(for:)` returns for SF.
            let line = (font.ascender() - font.descender() + font.leading()).round();
            (font.pointSize() as f32, weight(&font), line as f32)
        });
        SystemFonts {
            body: family(&system),
            rounded: rounded.as_deref().and_then(family),
            mono: family(&mono),
            text_styles: Some(TextStyles::from_rows(rows)),
        }
    }

    fn text_style(style: TextStyle) -> &'static NSFontTextStyle {
        unsafe {
            match style {
                TextStyle::LargeTitle => NSFontTextStyleLargeTitle,
                TextStyle::Title1 => NSFontTextStyleTitle1,
                TextStyle::Title2 => NSFontTextStyleTitle2,
                TextStyle::Title3 => NSFontTextStyleTitle3,
                TextStyle::Headline => NSFontTextStyleHeadline,
                TextStyle::Subheadline => NSFontTextStyleSubheadline,
                TextStyle::Body => NSFontTextStyleBody,
                TextStyle::Callout => NSFontTextStyleCallout,
                TextStyle::Footnote => NSFontTextStyleFootnote,
                TextStyle::Caption1 => NSFontTextStyleCaption1,
                TextStyle::Caption2 => NSFontTextStyleCaption2,
            }
        }
    }

    /// The font's CSS weight. A text style's font is an instance of SF's variable font whose
    /// weight trait reads 0 for Medium, so the weight axis is read first.
    fn weight(font: &Retained<NSFont>) -> f32 {
        // NSFont is toll-free bridged to CTFont, and CTFont's variation dictionary to
        // NSDictionary.
        let ct = unsafe { &*(Retained::as_ptr(font).cast::<CTFont>()) };
        let axis = unsafe { ct.variation() }.and_then(|variation| {
            let variation = unsafe {
                &*(objc2_core_foundation::CFRetained::as_ptr(&variation).as_ptr()
                    as *const NSDictionary<NSNumber, NSNumber>)
            };
            variation.objectForKey(&NSNumber::numberWithUnsignedInt(WEIGHT_AXIS))
        });
        match axis {
            Some(w) => w.doubleValue() as f32,
            None if font
                .fontDescriptor()
                .symbolicTraits()
                .contains(objc2_app_kit::NSFontDescriptorSymbolicTraits::TraitBold) =>
            {
                700.
            }
            None => 400.,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod desktop {
    use std::path::PathBuf;
    use std::process::Command;

    use super::SystemFonts;

    pub fn resolve() -> SystemFonts {
        let body: [fn() -> Option<String>; 3] =
            [|| dconf("font-name"), || settings_ini("gtk-font-name"), || gsettings("font-name")];
        let mono: [fn() -> Option<String>; 2] = [|| dconf("monospace-font-name"), || gsettings("monospace-font-name")];
        let pick = |sources: &[fn() -> Option<String>], generic: &str| {
            sources
                .iter()
                .find_map(|source| source().and_then(|d| pango_family(&d)).filter(|f| fontconfig::has_family(f)))
                .or_else(|| fontconfig::default_family(generic))
                .map(Into::into)
        };
        SystemFonts {
            body: pick(&body, "sans-serif"),
            rounded: None,
            mono: pick(&mono, "monospace"),
            text_styles: None,
        }
    }

    /// A GVariant string as `gsettings` and `dconf` print it: `'Adwaita Sans 11'`.
    fn unquote(output: std::process::Output) -> Option<String> {
        let text = String::from_utf8(output.stdout).ok().filter(|_| output.status.success())?;
        let text = text.trim().trim_matches('\'');
        (!text.is_empty()).then(|| text.to_owned())
    }

    /// A value the user set: dconf prints nothing for a key left at its default.
    fn dconf(key: &str) -> Option<String> {
        Command::new("dconf")
            .args(["read", &format!("/org/gnome/desktop/interface/{key}")])
            .output()
            .ok()
            .and_then(unquote)
    }

    /// The user's value or the desktop's default for it.
    fn gsettings(key: &str) -> Option<String> {
        Command::new("gsettings").args(["get", "org.gnome.desktop.interface", key]).output().ok().and_then(unquote)
    }

    /// GTK 4's `settings.ini`, then GTK 3's.
    fn settings_ini(key: &str) -> Option<String> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        ["gtk-4.0", "gtk-3.0"].iter().find_map(|dir| {
            let text = std::fs::read_to_string(config.join(dir).join("settings.ini")).ok()?;
            text.lines().find_map(|line| {
                let (k, v) = line.split_once('=')?;
                (k.trim() == key).then(|| v.trim().trim_matches('"').to_owned()).filter(|v| !v.is_empty())
            })
        })
    }

    /// Pango's style, weight, stretch and variant words, which follow the family in a font
    /// description.
    const STYLE_WORDS: &[&str] = &[
        "Normal",
        "Roman",
        "Oblique",
        "Italic",
        "Small-Caps",
        "All-Small-Caps",
        "Petite-Caps",
        "All-Petite-Caps",
        "Unicase",
        "Title-Caps",
        "Thin",
        "Ultra-Light",
        "Extra-Light",
        "Light",
        "Semi-Light",
        "Demi-Light",
        "Book",
        "Regular",
        "Medium",
        "Semi-Bold",
        "Demi-Bold",
        "Bold",
        "Ultra-Bold",
        "Extra-Bold",
        "Heavy",
        "Ultra-Heavy",
        "Extra-Heavy",
        "Ultra-Condensed",
        "Extra-Condensed",
        "Condensed",
        "Semi-Condensed",
        "Semi-Expanded",
        "Expanded",
        "Extra-Expanded",
        "Ultra-Expanded",
    ];

    /// The family of a Pango font description (`Adwaita Sans Bold 11`): the size and style
    /// words dropped, the first of a comma-separated list kept.
    pub(super) fn pango_family(description: &str) -> Option<String> {
        let family = description.split(',').next()?;
        let mut words: Vec<&str> = family.split_whitespace().collect();
        if words.last().is_some_and(|w| w.trim_end_matches("px").parse::<f32>().is_ok()) {
            words.pop();
        }
        while words.len() > 1 && words.last().is_some_and(|w| STYLE_WORDS.iter().any(|s| s.eq_ignore_ascii_case(w))) {
            words.pop();
        }
        let family = words.join(" ");
        (!family.is_empty()).then_some(family)
    }

    mod fontconfig {
        use std::ffi::{CStr, CString};

        use fontconfig_sys::constants::FC_FAMILY;
        use fontconfig_sys::{
            FcChar8, FcConfigSubstitute, FcDefaultSubstitute, FcFontMatch, FcMatchPattern, FcPatternAddString,
            FcPatternCreate, FcPatternDestroy, FcPatternGetString, FcResultMatch, FcResultNoMatch,
        };

        /// Every family name on fontconfig's best match for `family`.
        fn matched(family: &str) -> Vec<String> {
            let Ok(name) = CString::new(family) else { return Vec::new() };
            let mut families = Vec::new();
            unsafe {
                let pattern = FcPatternCreate();
                if pattern.is_null() {
                    return families;
                }
                FcPatternAddString(pattern, FC_FAMILY.as_ptr(), name.as_ptr().cast::<FcChar8>());
                FcConfigSubstitute(std::ptr::null_mut(), pattern, FcMatchPattern);
                FcDefaultSubstitute(pattern);
                let mut result = FcResultNoMatch;
                let font = FcFontMatch(std::ptr::null_mut(), pattern, &mut result);
                FcPatternDestroy(pattern);
                if font.is_null() {
                    return families;
                }
                let mut value: *mut FcChar8 = std::ptr::null_mut();
                let mut index = 0;
                while FcPatternGetString(font, FC_FAMILY.as_ptr(), index, &mut value) == FcResultMatch {
                    families.push(CStr::from_ptr(value.cast()).to_string_lossy().into_owned());
                    index += 1;
                }
                FcPatternDestroy(font);
            }
            families
        }

        /// What fontconfig substitutes for a generic family (`sans-serif`, `monospace`).
        pub fn default_family(generic: &str) -> Option<String> {
            matched(generic).into_iter().next()
        }

        /// Whether `family` is installed, rather than substituted.
        pub fn has_family(family: &str) -> bool {
            matched(family).iter().any(|f| f.eq_ignore_ascii_case(family))
        }
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "freebsd")))]
mod tests {
    use super::desktop::pango_family;

    #[test]
    fn pango_descriptions_give_their_family() {
        assert_eq!(pango_family("Adwaita Sans 11").as_deref(), Some("Adwaita Sans"));
        assert_eq!(pango_family("GeistMono Nerd Font 11").as_deref(), Some("GeistMono Nerd Font"));
        assert_eq!(pango_family("Cantarell Bold Italic 10.5").as_deref(), Some("Cantarell"));
        assert_eq!(pango_family("Inter, Noto Sans 12px").as_deref(), Some("Inter"));
        assert_eq!(pango_family("Monospace").as_deref(), Some("Monospace"));
        assert_eq!(pango_family("11"), None);
    }
}
