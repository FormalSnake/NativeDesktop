//! What a rich text run's colour token is under a theme, for every backend.

use nd_theme::Theme;
use nd_ui::TextColor;
use nd_ui::prelude::Hsla;

/// The colour `color` names under `theme`. `ink` is the ink of a filled surface the text sits
/// on (a sent bubble): every token but a custom colour resolves to it there, since the
/// surface's own ink is the only colour that reads on it at 4.5:1.
pub fn text_color(theme: &Theme, color: TextColor, ink: Option<Hsla>) -> Hsla {
    if let (Some(ink), false) = (ink, matches!(color, TextColor::Custom(_))) {
        return ink;
    }
    match color {
        TextColor::Primary => theme.text.primary,
        TextColor::Secondary => theme.text.secondary,
        TextColor::Tertiary => theme.text.tertiary,
        TextColor::Accent => theme.text.accent,
        TextColor::Danger => theme.text.danger,
        TextColor::Success => theme.text.success,
        TextColor::Warning => theme.text.warning,
        TextColor::Custom(c) => c,
    }
}
