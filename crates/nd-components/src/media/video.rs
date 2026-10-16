//! `video(source)`: a playing video inside the window, from a file path or URL. The app owns
//! play state: `playing`, `muted` and `looping` are props, and the node reports when the media
//! is ready, when it ends and when it fails. Size it with the style; `fit` crops or letterboxes
//! the picture into that box.
//!
//! On AppKit the picture is an `AVPlayerLayer`. Backends without a video surface draw the
//! placeholder fill and report [`VideoFailed`], so the app can fall back to the system player.

use std::panic::Location;

use nd_ui::prelude::*;
use nd_ui::semantics::{Semantics, WidgetSemantics};
use nd_ui::{Key, Role, StyleRefinement};

use super::pipeline::ImageFit;
use crate::forms::leaf;

#[derive(Clone, Debug, PartialEq)]
pub struct VideoProps {
    /// A file path or a URL; `None` shows the placeholder only.
    pub source: Option<SharedString>,
    pub playing: bool,
    pub muted: bool,
    /// Starts again from the beginning at the end, without reporting [`VideoEnded`].
    pub looping: bool,
    pub fit: ImageFit,
    /// What the video shows, for the accessibility tree.
    pub alt: Option<SharedString>,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    pub style: StyleRefinement,
}

/// The media opened: its picture size in pixels and its length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoReady {
    pub width: u32,
    pub height: u32,
    /// `None` for a live stream.
    pub duration: Option<f64>,
}

/// Playback reached the end (never sent while looping).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoEnded;

/// The media could not be opened or played, or the backend has no video surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoFailed {
    pub error: String,
}

/// Widget kind of `video`.
pub struct VideoKind;

impl WidgetSemantics for VideoProps {
    fn semantics(&self) -> Semantics {
        let name = self.alt.clone().unwrap_or_default();
        Semantics::new(Role::Image, name.to_string()).enabled(self.enabled)
    }
}

pub struct Video(NativeElement<VideoProps>);

/// A video from a path or URL, paused until [`Video::playing`] is set.
#[track_caller]
pub fn video(source: impl Into<SharedString>) -> Video {
    let props = VideoProps {
        source: Some(source.into()),
        playing: false,
        muted: false,
        looping: false,
        fit: ImageFit::Contain,
        alt: None,
        enabled: true,
        tooltip: None,
        style: StyleRefinement::default(),
    };
    Video(NativeElement::described::<VideoKind>(Key::Site(Location::caller()), props))
}

impl Video {
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        *self.0.key_mut() = Key::Id(id.into());
        self
    }

    pub fn playing(mut self, playing: bool) -> Self {
        self.0.props_mut().playing = playing;
        self
    }

    pub fn muted(mut self, muted: bool) -> Self {
        self.0.props_mut().muted = muted;
        self
    }

    pub fn looping(mut self, looping: bool) -> Self {
        self.0.props_mut().looping = looping;
        self
    }

    /// Crops the picture to fill the box instead of letterboxing it.
    pub fn cover(mut self) -> Self {
        self.0.props_mut().fit = ImageFit::Cover;
        self
    }

    pub fn alt(mut self, alt: impl Into<SharedString>) -> Self {
        self.0.props_mut().alt = Some(alt.into());
        self
    }

    pub fn on_ready(self, f: impl Fn(&VideoReady, &mut Window, &mut App) + 'static) -> Self {
        Video(self.0.on(f))
    }

    pub fn on_end(self, f: impl Fn(&VideoEnded, &mut Window, &mut App) + 'static) -> Self {
        Video(self.0.on(f))
    }

    pub fn on_error(self, f: impl Fn(&VideoFailed, &mut Window, &mut App) + 'static) -> Self {
        Video(self.0.on(f))
    }
}

leaf!(Video, VideoProps);
