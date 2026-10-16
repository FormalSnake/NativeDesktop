//! The only place this app names colours. `nd check` allows colour literals in theme files.

use nd::prelude::*;

pub fn app() -> ThemeOverride {
    ThemeOverride::new().accent(rgb(0x3d6bff)).radius_scale(1.25)
}

pub fn warm(theme: ThemeRefinement) -> ThemeRefinement {
    theme.accent(rgb(0xff8a3d)).radius_scale(1.5)
}

pub fn warm_glass_tint() -> Hsla {
    rgba(0xff8a3d33).into()
}

pub fn forest() -> Hsla {
    rgb(0x2f6f4f).into()
}

pub fn forest_ink() -> Hsla {
    rgb(0xffffff).into()
}
