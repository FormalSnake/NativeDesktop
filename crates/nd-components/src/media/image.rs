//! `image(source)`: a picture from the [pipeline](super::pipeline), decoded off the main thread
//! at the size it is shown. Until it decodes the node shows a placeholder of its final size, so
//! rows never jump. Rounded corners come from the element's style (`.rounded_lg()`,
//! `.rounded_full()` for a circle) and clip the picture. Animated GIF and WebP play while
//! `animate` is on and the OS does not ask for reduced motion.
//!
//! On AppKit the picture is the contents of a layer-backed view (ARCHITECTURE.md 3.7); on GPUI
//! backends it is a GPUI image with the same pipeline behind it.

use std::panic::Location;

use nd_core::Size;
use nd_ui::prelude::*;
use nd_ui::semantics::{Semantics, WidgetSemantics};
use nd_ui::{Key, Role, StyleRefinement};

use super::chat::BubbleSide;
use super::pipeline::{ImageFit, ImagePipeline, ImageSource};
use crate::forms::leaf;

/// The aspect a placeholder takes when nothing tells it the image's own: 4:3, as a photo.
pub const UNKNOWN_ASPECT: f32 = 4. / 3.;

#[derive(Clone, Debug, PartialEq)]
pub struct ImageProps {
    /// `None` shows the placeholder only.
    pub source: Option<ImageSource>,
    pub fit: ImageFit,
    /// The image's size in pixels when the app already knows it (attachment metadata), so the
    /// placeholder takes the final shape before the header is read.
    pub size_hint: Option<(u32, u32)>,
    /// What the picture shows, for the accessibility tree.
    pub alt: Option<SharedString>,
    /// Plays animated images; they show their first frame otherwise.
    pub animate: bool,
    /// Draws every pixel in this colour, keeping its alpha: a monochrome icon in a token.
    pub tint: Option<Hsla>,
    /// Cut to a message bubble with its tail on this side: a picture ending a run of messages.
    pub tail: Option<BubbleSide>,
    /// Softens the picture to colour washes about this many points across: decoded that small
    /// and stretched, the same on every backend, for a backdrop behind content.
    pub blur: Option<f32>,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    pub style: StyleRefinement,
}

impl ImageProps {
    /// The pixels per point to decode at: the screen's, or far fewer for a blurred picture.
    pub fn decode_scale(&self, scale: f32) -> f32 {
        match self.blur {
            Some(radius) if radius > 1. => scale.min(1. / radius),
            _ => scale,
        }
    }

    /// The image's own pixel size: the hint, else the header.
    pub fn intrinsic(&self, pipeline: &ImagePipeline) -> Option<(u32, u32)> {
        self.size_hint.or_else(|| self.source.as_ref().and_then(|s| pipeline.intrinsic_size(s)))
    }

    /// The size the node asks for at `width`: its own aspect, never wider than its pixels.
    /// Taffy only asks when the style leaves a side open.
    pub fn natural_size(&self, width: Option<f32>, pipeline: &ImagePipeline) -> Size {
        let (w, h) = match self.intrinsic(pipeline) {
            Some((w, h)) if w > 0 && h > 0 => (w as f32, h as f32),
            _ => (UNKNOWN_ASPECT * 240., 240.),
        };
        let shown = width.map_or(w, |avail| avail.min(w));
        Size { width: shown, height: (shown * h / w).round() }
    }
}

/// The picture decoded and is on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageLoaded {
    pub width: u32,
    pub height: u32,
    pub animated: bool,
}

/// The picture could not be loaded or decoded. It is not retried for
/// [`RETRY_AFTER`](super::pipeline::RETRY_AFTER).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageFailed {
    pub error: String,
}

/// Widget kind of `image`.
pub struct ImageKind;

impl WidgetSemantics for ImageProps {
    fn semantics(&self) -> Semantics {
        let name = self.alt.clone().unwrap_or_default();
        Semantics::new(Role::Image, name.to_string()).enabled(self.enabled)
    }
}

pub struct Image(NativeElement<ImageProps>);

/// A picture from a path, URL or bytes. Size it with the style; a side left open follows the
/// image's aspect.
#[track_caller]
pub fn image(source: impl Into<ImageSource>) -> Image {
    image_at(Some(source.into()), Location::caller())
}

/// An image node with no picture yet: the placeholder alone, at the size the style gives.
#[track_caller]
pub fn image_placeholder() -> Image {
    image_at(None, Location::caller())
}

fn image_at(source: Option<ImageSource>, site: &'static Location<'static>) -> Image {
    let props = ImageProps {
        source,
        fit: ImageFit::Cover,
        size_hint: None,
        alt: None,
        animate: true,
        tint: None,
        tail: None,
        blur: None,
        enabled: true,
        tooltip: None,
        style: StyleRefinement::default(),
    };
    Image(NativeElement::described::<ImageKind>(Key::Site(site), props))
}

impl Image {
    /// Cuts the picture into a message bubble whose tail leaves the bottom corner on `side`'s
    /// outer edge, the picture running into the tail, as Messages draws a photo that ends a run.
    /// The tail takes [`BUBBLE.tail_drop`](super::BUBBLE) of the element's height at its
    /// bottom; the corner radius is the style's. GPUI backends draw the rounded picture without the
    /// tail.
    pub fn bubble_tail(mut self, side: BubbleSide) -> Self {
        self.0.props_mut().tail = Some(side);
        self
    }

    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        *self.0.key_mut() = Key::Id(id.into());
        self
    }

    pub fn fit(mut self, fit: ImageFit) -> Self {
        self.0.props_mut().fit = fit;
        self
    }

    /// Softens the picture to washes of its colours about `radius` across, for a backdrop.
    pub fn blur(mut self, radius: Pixels) -> Self {
        self.0.props_mut().blur = Some(radius.as_f32());
        self
    }

    /// Draws the picture in `color` (a theme token), keeping each pixel's alpha.
    pub fn tint(mut self, color: impl Into<Hsla>) -> Self {
        self.0.props_mut().tint = Some(color.into());
        self
    }

    /// Letterboxes instead of cropping.
    pub fn contain(self) -> Self {
        self.fit(ImageFit::Contain)
    }

    pub fn size_hint(mut self, width: u32, height: u32) -> Self {
        self.0.props_mut().size_hint = Some((width, height));
        self
    }

    pub fn alt(mut self, alt: impl Into<SharedString>) -> Self {
        self.0.props_mut().alt = Some(alt.into());
        self
    }

    /// Shows the first frame of an animation only.
    pub fn still(mut self) -> Self {
        self.0.props_mut().animate = false;
        self
    }

    pub fn on_load(self, f: impl Fn(&ImageLoaded, &mut Window, &mut App) + 'static) -> Self {
        Image(self.0.on(f))
    }

    pub fn on_error(self, f: impl Fn(&ImageFailed, &mut Window, &mut App) + 'static) -> Self {
        Image(self.0.on(f))
    }
}

leaf!(Image, ImageProps);
