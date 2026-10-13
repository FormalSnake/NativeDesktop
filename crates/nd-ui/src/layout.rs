//! GPUI style refinements to Taffy styles, mirroring GPUI's own mapping so a tree lays out
//! the same on retained backends as inside GPUI. Rounding to the pixel grid is the backend's.

use nd_gpui::gpui::{self, AbsoluteLength, DefiniteLength, Edges, Length, Refineable, StyleRefinement, px};

use crate::window::TitlebarInsets;
use taffy::style::{Dimension, LengthPercentage, LengthPercentageAuto};

const REM: f32 = 16.;

fn abs(l: AbsoluteLength) -> f32 {
    l.to_pixels(px(REM)).as_f32()
}

fn lp(l: DefiniteLength) -> LengthPercentage {
    match l {
        DefiniteLength::Absolute(a) => LengthPercentage::length(abs(a)),
        DefiniteLength::Fraction(f) => LengthPercentage::percent(f),
    }
}

fn lpa(l: Length) -> LengthPercentageAuto {
    match l {
        Length::Definite(DefiniteLength::Absolute(a)) => LengthPercentageAuto::length(abs(a)),
        Length::Definite(DefiniteLength::Fraction(f)) => LengthPercentageAuto::percent(f),
        Length::Auto => LengthPercentageAuto::auto(),
    }
}

fn dim(l: Length) -> Dimension {
    match l {
        Length::Definite(DefiniteLength::Absolute(a)) => Dimension::length(abs(a)),
        Length::Definite(DefiniteLength::Fraction(f)) => Dimension::percent(f),
        Length::Auto => Dimension::auto(),
    }
}

fn rect<T: Clone + std::fmt::Debug + Default + PartialEq, U>(e: &Edges<T>, f: impl Fn(T) -> U) -> taffy::Rect<U> {
    taffy::Rect {
        top: f(e.top.clone()),
        right: f(e.right.clone()),
        bottom: f(e.bottom.clone()),
        left: f(e.left.clone()),
    }
}

fn size<T: Clone + std::fmt::Debug + Default + PartialEq, U>(s: &gpui::Size<T>, f: impl Fn(T) -> U) -> taffy::Size<U> {
    taffy::Size { width: f(s.width.clone()), height: f(s.height.clone()) }
}

pub(crate) fn to_taffy(refinement: &StyleRefinement) -> taffy::Style {
    let mut s = gpui::Style::default();
    s.refine(refinement);
    taffy::Style {
        display: s.display.into(),
        overflow: taffy::Point { x: s.overflow.x.into(), y: s.overflow.y.into() },
        position: s.position.into(),
        inset: rect(&s.inset, lpa),
        size: size(&s.size, dim),
        min_size: size(&s.min_size, dim),
        max_size: size(&s.max_size, dim),
        aspect_ratio: s.aspect_ratio,
        margin: rect(&s.margin, lpa),
        padding: rect(&s.padding, lp),
        border: rect(&s.border_widths, |w| LengthPercentage::length(abs(w))),
        align_items: s.align_items.map(Into::into),
        align_self: s.align_self.map(Into::into),
        align_content: s.align_content.map(Into::into),
        justify_content: s.justify_content.map(Into::into),
        gap: size(&s.gap, lp),
        flex_direction: s.flex_direction.into(),
        flex_wrap: s.flex_wrap.into(),
        flex_basis: dim(s.flex_basis),
        flex_grow: s.flex_grow,
        flex_shrink: s.flex_shrink,
        ..Default::default()
    }
}

/// `style` padded by the titlebar insets: leading and trailing added to the left and right
/// padding, the height as a minimum. Every backend lays a `titlebar_insets()` element out
/// through this, so the controls' room is the same on retained and GPUI backends.
pub fn with_titlebar_insets(style: &StyleRefinement, insets: TitlebarInsets) -> StyleRefinement {
    let pad = |l: Option<DefiniteLength>, add: f32| match l {
        Some(DefiniteLength::Fraction(f)) if add == 0. => Some(DefiniteLength::Fraction(f)),
        Some(DefiniteLength::Absolute(a)) => Some(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(abs(a) + add)))),
        // A fractional padding has no point size to add to; the inset wins.
        Some(DefiniteLength::Fraction(_)) | None => Some(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(add)))),
    };
    let mut out = style.clone();
    out.padding.left = pad(style.padding.left, insets.leading);
    out.padding.right = pad(style.padding.right, insets.trailing);
    let min = match style.min_size.height {
        Some(Length::Definite(DefiniteLength::Absolute(a))) => abs(a).max(insets.height),
        _ => insets.height,
    };
    out.min_size.height = Some(Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(min)))));
    out
}
