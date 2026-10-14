//! The icons nd names, one vocabulary on every backend. Each [`Icon`] is a Lucide name:
//! macOS draws the matching SF Symbol (the first candidate the OS has, so a newer symbol can
//! be listed ahead of its macOS 14 stand-in), and the GPUI backends and the web draw the
//! vendored Lucide shape, tinted by the theme. Apps add their own icons with [`register`],
//! which gives them the same treatment.
//!
//! The Lucide SVGs live in `lucide/` (pinned by `scripts/vendor-lucide`, license included) and
//! `build.rs` bakes them into [`Path`] data, so nothing parses SVG at run time except an app's
//! own.

mod svg;

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

pub use svg::Seg;

/// One drawn element of an icon: stroked, or filled when the SVG asks for it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Path {
    pub filled: bool,
    pub segs: &'static [Seg],
}

/// An icon's drawing: its paths on a square grid `size` units wide, stroked 2 units thick with
/// round caps and joins (Lucide's convention).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub paths: &'static [Path],
    pub size: f32,
}

/// Lucide's stroke width on its 24 unit grid.
pub const STROKE: f32 = 2.;

use Seg::{Close as Z, Cubic as C, Line as L, Move as M};

static LUCIDE: &[(&str, &[Path])] = include!(concat!(env!("OUT_DIR"), "/lucide.rs"));

macro_rules! icons {
    ($($variant:ident $lucide:literal [$($sf:literal),+];)+) => {
        /// A named icon: one of nd's own, or an app's registered through [`register`].
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Icon {
            $($variant,)+
            /// An icon from [`register`], by its Lucide-style name.
            Custom(&'static str),
        }

        impl Icon {
            /// Every icon nd ships.
            pub const ALL: &'static [Icon] = &[$(Icon::$variant,)+];

            /// The Lucide name, which is what crosses to backends.
            pub fn lucide(self) -> &'static str {
                match self {
                    $(Icon::$variant => $lucide,)+
                    Icon::Custom(name) => name,
                }
            }

            fn builtin_symbols(self) -> &'static [&'static str] {
                match self {
                    $(Icon::$variant => &[$($sf),+],)+
                    Icon::Custom(_) => &[],
                }
            }

            fn builtin(name: &str) -> Option<Icon> {
                match name {
                    $($lucide => Some(Icon::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

icons! {
    Archive "archive" ["archivebox"];
    ArrowUp "arrow-up" ["arrow.up"];
    Calendar "calendar" ["calendar"];
    Check "check" ["checkmark"];
    ChevronDown "chevron-down" ["chevron.down"];
    ChevronLeft "chevron-left" ["chevron.left"];
    ChevronRight "chevron-right" ["chevron.right"];
    ChevronUp "chevron-up" ["chevron.up"];
    Circle "circle" ["circle"];
    CircleCheck "circle-check" ["checkmark.circle"];
    CircleX "circle-x" ["xmark.circle.fill"];
    Clock "clock" ["clock"];
    Code "code" ["chevron.left.forwardslash.chevron.right"];
    Download "download" ["arrow.down.circle"];
    Eye "eye" ["eye"];
    EyeOff "eye-off" ["eye.slash"];
    FileText "file-text" ["doc.text"];
    Folder "folder" ["folder"];
    Globe "globe" ["globe"];
    Image "image" ["photo"];
    Inbox "inbox" ["tray"];
    Info "info" ["info.circle"];
    LocateFixed "locate-fixed" ["location.fill"];
    Lock "lock" ["lock"];
    Mail "mail" ["envelope"];
    MapPin "map-pin" ["mappin"];
    Minus "minus" ["minus"];
    Music "music" ["music.note"];
    Notebook "notebook" ["book.closed"];
    PanelLeft "panel-left" ["sidebar.left"];
    PanelRight "panel-right" ["sidebar.right"];
    Paperclip "paperclip" ["paperclip"];
    Pause "pause" ["pause.fill"];
    Pin "pin" ["pin"];
    Play "play" ["play.fill"];
    Plus "plus" ["plus"];
    RotateCw "rotate-cw" ["arrow.clockwise"];
    Search "search" ["magnifyingglass"];
    Settings "settings" ["gearshape"];
    Share "share" ["square.and.arrow.up"];
    SkipBack "skip-back" ["backward.fill"];
    SkipForward "skip-forward" ["forward.fill"];
    Smile "smile" ["face.smiling", "smiley"];
    SquarePen "square-pen" ["square.and.pencil"];
    Star "star" ["star"];
    Trash "trash" ["trash"];
    TriangleAlert "triangle-alert" ["exclamationmark.triangle"];
    User "user" ["person"];
    Volume "volume" ["speaker.fill"];
    Volume1 "volume-1" ["speaker.wave.1.fill"];
    Volume2 "volume-2" ["speaker.wave.3.fill"];
    VolumeX "volume-x" ["speaker.slash.fill"];
    X "x" ["xmark"];
}

struct Custom {
    shape: Shape,
    symbols: &'static [&'static str],
}

fn custom() -> &'static RwLock<HashMap<&'static str, Custom>> {
    static CUSTOM: OnceLock<RwLock<HashMap<&'static str, Custom>>> = OnceLock::new();
    CUSTOM.get_or_init(Default::default)
}

/// Adds an app's own icon: `svg` is a Lucide-style SVG (square viewBox from 0 0, strokes drawn
/// 2 units thick, no `<text>`, gradients or transforms), `symbols` the SF Symbols macOS should
/// prefer, best first (empty to draw the SVG there too). A name nd already ships, or one
/// registered before, is an error. The returned icon works anywhere an [`Icon`] does.
pub fn register(name: &str, svg: &str, symbols: &[&'static str]) -> Result<Icon, String> {
    if Icon::builtin(name).is_some() {
        return Err(format!("`{name}` is one of nd's own icons"));
    }
    let (pieces, size) = svg::parse(svg)?;
    let mut map = custom().write().expect("icon registry lock");
    if map.contains_key(name) {
        return Err(format!("`{name}` is already registered"));
    }
    let name: &'static str = Box::leak(name.to_owned().into_boxed_str());
    let paths: Vec<Path> = pieces
        .into_iter()
        .map(|p| Path { filled: p.filled, segs: Box::leak(p.segs.into_boxed_slice()) })
        .collect();
    let shape = Shape { paths: Box::leak(paths.into_boxed_slice()), size };
    let symbols: &'static [&'static str] = Box::leak(symbols.to_vec().into_boxed_slice());
    map.insert(name, Custom { shape, symbols });
    Ok(Icon::Custom(name))
}

impl Icon {
    /// The icon for a Lucide name, ours or registered.
    pub fn from_name(name: &str) -> Option<Icon> {
        Icon::builtin(name).or_else(|| {
            custom().read().expect("icon registry lock").get_key_value(name).map(|(k, _)| Icon::Custom(k))
        })
    }

    /// The drawing every GPUI backend and the web paints.
    pub fn shape(self) -> Option<Shape> {
        match self {
            Icon::Custom(name) => custom().read().expect("icon registry lock").get(name).map(|c| c.shape),
            _ => {
                let at = LUCIDE.binary_search_by_key(&self.lucide(), |(n, _)| n).ok()?;
                Some(Shape { paths: LUCIDE[at].1, size: 24. })
            }
        }
    }

    /// SF Symbol names for macOS, best first; the backend uses the first the OS knows. Empty
    /// for a custom icon with no symbol, which macOS then draws from [`Icon::shape`].
    pub fn sf_symbols(self) -> &'static [&'static str] {
        match self {
            Icon::Custom(name) => custom().read().expect("icon registry lock").get(name).map_or(&[], |c| c.symbols),
            _ => self.builtin_symbols(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_has_a_shape_and_a_symbol() {
        for icon in Icon::ALL {
            let shape = icon.shape().unwrap_or_else(|| panic!("{} has no lucide shape", icon.lucide()));
            assert!(shape.paths.iter().all(|p| !p.segs.is_empty()), "{} has an empty path", icon.lucide());
            assert!(!icon.sf_symbols().is_empty(), "{} has no SF Symbol", icon.lucide());
            assert_eq!(Icon::from_name(icon.lucide()), Some(*icon));
        }
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<_> = Icon::ALL.iter().map(|i| i.lucide()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), Icon::ALL.len());
    }

    #[test]
    fn the_vendored_set_is_exactly_the_enum() {
        let mut vendored: Vec<_> = LUCIDE.iter().map(|(n, _)| *n).collect();
        let mut named: Vec<_> = Icon::ALL.iter().map(|i| i.lucide()).collect();
        vendored.sort();
        named.sort();
        assert_eq!(vendored, named);
    }

    #[test]
    fn shapes_stay_on_the_grid() {
        for icon in Icon::ALL {
            let shape = icon.shape().unwrap();
            for path in shape.paths {
                for seg in path.segs {
                    let pts: Vec<f32> = match *seg {
                        Seg::Move(x, y) | Seg::Line(x, y) => vec![x, y],
                        Seg::Cubic(a, b, c, d, e, f) => vec![a, b, c, d, e, f],
                        Seg::Close => vec![],
                    };
                    assert!(pts.iter().all(|v| (-1. ..=shape.size + 1.).contains(v)), "{}: {seg:?}", icon.lucide());
                }
            }
        }
    }

    #[test]
    fn a_registered_icon_resolves_like_a_built_in() {
        let svg = r#"<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 3"/></svg>"#;
        let icon = register("test-clock-face", svg, &["clock.fill"]).unwrap();
        assert_eq!(icon.lucide(), "test-clock-face");
        assert_eq!(Icon::from_name("test-clock-face"), Some(icon));
        assert_eq!(icon.shape().unwrap().paths.len(), 2);
        assert_eq!(icon.sf_symbols(), ["clock.fill"]);
        assert!(register("test-clock-face", svg, &[]).is_err());
        assert!(register("check", svg, &[]).is_err());
    }

    #[test]
    fn arcs_close_on_their_endpoint() {
        let (pieces, _) = svg::parse(r#"<svg viewBox="0 0 24 24"><path d="M2 12a10 10 0 0 1 20 0"/></svg>"#).unwrap();
        let Some(Seg::Cubic(.., x, y)) = pieces[0].segs.last().copied() else { panic!("no cubic") };
        assert!((x - 22.).abs() < 1e-3 && (y - 12.).abs() < 1e-3);
    }
}
