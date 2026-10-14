//! Noto Sans and Noto Sans Mono, the plain preset's families (`nd_theme::Preset::Plain`), under
//! the SIL Open Font License (`assets/fonts/OFL.txt`). Four sans weights cover the type scale;
//! cosmic-text synthesizes nothing, so a missing weight would fall back to the nearest one.

use std::borrow::Cow;

use nd_gpui::gpui;

const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/NotoSans-Regular.ttf"),
    include_bytes!("../assets/fonts/NotoSans-Medium.ttf"),
    include_bytes!("../assets/fonts/NotoSans-SemiBold.ttf"),
    include_bytes!("../assets/fonts/NotoSans-Bold.ttf"),
    include_bytes!("../assets/fonts/NotoSansMono-Regular.ttf"),
    include_bytes!("../assets/fonts/NotoSansMono-Bold.ttf"),
];

pub fn register(cx: &mut gpui::App) {
    let fonts = FONTS.iter().map(|bytes| Cow::Borrowed(*bytes)).collect();
    if let Err(error) = cx.text_system().add_fonts(fonts) {
        crate::console_error(&format!("nd: bundled fonts failed to load: {error:#}"));
    }
}
