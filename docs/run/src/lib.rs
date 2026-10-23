//! The web examples the docs run live: on the site in this crate's own bundle (`/run/`), in the
//! docs app in its own process. Their windows are in-canvas windows on the web, native windows on
//! the desktop.

use nd::prelude::*;

pub fn app() -> Application {
    Application::new(nd::manifest!())
}

/// Opens example `name`'s window, for every example with `web = true` in docs/examples.toml.
pub fn opener(name: &str) -> Option<fn(&mut App)> {
    let open: fn(&mut App) = match name {
        "hello" => |cx| {
            hello::open(cx);
        },
        "gallery" => |cx| {
            gallery::open(cx, gallery::Start::default());
        },
        "overrides" => |cx| {
            overrides::open(cx, Appearance::System);
        },
        // The capture runs 100 000 rows; a page embed keeps it light.
        "scroll" => |cx| {
            scroll_bench::open(cx, 2_000, None);
        },
        _ => return None,
    };
    Some(open)
}
