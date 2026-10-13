//! System colours resolved for the app's current appearance, for the theme preset that composed
//! content reads (ARCHITECTURE.md 3.10). Native views resolve their own colours at draw time.

use std::cell::Cell;

use block2::RcBlock;
use nd_ui::prelude::{Hsla, Rgba};
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSColor, NSColorSpace};
use objc2_foundation::MainThreadMarker;

pub struct SystemColors {
    pub label: Hsla,
    pub secondary_label: Hsla,
    pub tertiary_label: Hsla,
    pub accent: Hsla,
}

fn hsla(color: Retained<NSColor>) -> Hsla {
    let Some(c) = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace()) else {
        return Hsla::default();
    };
    Rgba {
        r: c.redComponent() as f32,
        g: c.greenComponent() as f32,
        b: c.blueComponent() as f32,
        a: c.alphaComponent() as f32,
    }
    .into()
}

/// Resolves under `NSApp.effectiveAppearance`, so light and dark give different values.
pub fn system_colors(mtm: MainThreadMarker) -> SystemColors {
    let out = Cell::new(None);
    let block = RcBlock::new(|| {
        out.set(Some(SystemColors {
            label: hsla(NSColor::labelColor()),
            secondary_label: hsla(NSColor::secondaryLabelColor()),
            tertiary_label: hsla(NSColor::tertiaryLabelColor()),
            accent: hsla(NSColor::controlAccentColor()),
        }));
    });
    NSApplication::sharedApplication(mtm).effectiveAppearance().performAsCurrentDrawingAppearance(&block);
    out.take().expect("the block ran synchronously")
}
