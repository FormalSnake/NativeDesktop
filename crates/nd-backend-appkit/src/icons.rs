//! Lucide names to images, so icons are the system's own on macOS: the first SF Symbol the OS
//! knows from the icon's candidates (a newer symbol can precede its macOS 14 stand-in), else
//! the icon's Lucide shape drawn as a template image, which is how an app's own icons arrive.

use block2::RcBlock;
use nd_icons::{Icon, Seg};
use objc2::rc::Retained;
use objc2_app_kit::{NSBezierPath, NSColor, NSImage, NSLineCapStyle, NSLineJoinStyle};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use objc2::runtime::Bool;

/// Side of the drawn fallback, in points; an image view scales it to fit.
const DRAWN: f64 = 24.;

/// The image for a Lucide name; `None` when it names no icon nd or the app registered.
pub(crate) fn symbol(lucide: &str) -> Option<Retained<NSImage>> {
    let icon = Icon::from_name(lucide)?;
    symbol_name(lucide)
        .and_then(|name| NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None))
        .or_else(|| drawn(icon))
}

/// The SF Symbol this OS has for a Lucide name, for SwiftUI's `systemImage:`.
pub(crate) fn symbol_name(lucide: &str) -> Option<&'static str> {
    Icon::from_name(lucide)?.sf_symbols().iter().copied().find(|name| {
        NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None).is_some()
    })
}

fn drawn(icon: Icon) -> Option<Retained<NSImage>> {
    let shape = icon.shape()?;
    let scale = DRAWN / shape.size as f64;
    let handler = RcBlock::new(move |_: NSRect| -> Bool {
        NSColor::blackColor().set();
        for piece in shape.paths {
            let path = NSBezierPath::bezierPath();
            let at = |x: f32, y: f32| NSPoint::new(x as f64 * scale, y as f64 * scale);
            for seg in piece.segs {
                match *seg {
                    Seg::Move(x, y) => path.moveToPoint(at(x, y)),
                    Seg::Line(x, y) => path.lineToPoint(at(x, y)),
                    Seg::Cubic(a, b, c, d, x, y) => path.curveToPoint_controlPoint1_controlPoint2(at(x, y), at(a, b), at(c, d)),
                    Seg::Close => path.closePath(),
                }
            }
            if piece.filled {
                path.fill();
            } else {
                path.setLineWidth(nd_icons::STROKE as f64 * scale);
                path.setLineCapStyle(NSLineCapStyle::Round);
                path.setLineJoinStyle(NSLineJoinStyle::Round);
                path.stroke();
            }
        }
        Bool::YES
    });
    let image = NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(DRAWN, DRAWN), true, &handler);
    image.setTemplate(true);
    Some(image)
}

#[cfg(test)]
mod tests {
    use nd_icons::Icon;

    use super::*;

    #[test]
    fn every_icon_has_a_system_symbol() {
        for icon in Icon::ALL {
            assert!(symbol_name(icon.lucide()).is_some(), "{} has no SF Symbol on this macOS", icon.lucide());
            assert!(symbol(icon.lucide()).is_some());
        }
    }

    #[test]
    fn a_custom_icon_without_a_symbol_is_drawn() {
        let svg = r#"<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="9"/></svg>"#;
        let icon = nd_icons::register("appkit-test-ring", svg, &[]).unwrap();
        assert!(symbol_name(icon.lucide()).is_none());
        assert!(symbol(icon.lucide()).is_some_and(|image| image.isTemplate()));
    }

    #[test]
    fn an_unknown_name_resolves_to_nothing() {
        assert!(symbol("no-such-icon").is_none());
    }
}
