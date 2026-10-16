//! Media and content (M6): the image pipeline and `image()`, media transport (`seek_bar`,
//! `player_bar`), synced lyrics, the sideways `shelf`, the chat `composer`, `chat_bubble`
//! rows, `avatar`s and `video`.
//!
//! Every rule lives here, backend-neutral and tested on `TestBackend`; backends draw the kinds
//! and turn input into the models' calls. Where the platform has the control (a text view, a
//! button, a slider, a scroll view) the composed pieces use it; the seek bar, lyrics, bubbles
//! and avatars have no system control anywhere and are drawn from theme tokens.

mod chat;
mod clock;
mod composer;
mod image;
mod lyrics;
pub mod pipeline;
mod shelf;
mod transport;
mod video;

pub use chat::{
    AVATAR_COLUMN, AVATAR_GROUP_SCALE, Avatar, AvatarKind, AvatarProps, BUBBLE, BubbleMetrics, BubbleSide, ChatBubble,
    ChatBubbleKind, ChatBubbleProps, ChatMessage, MONOGRAM_MIN, Person, Reaction, RunPosition, avatar, avatar_group,
    bubble_corners, chat_bubble, chat_row, group_layout, initials, is_emoji_only, monogram_size,
};
pub use clock::{PlaybackClock, format_clock, parse_clock};
pub use composer::{ATTACHMENT_THUMB, Attachment, COMPOSER_LINES, Composer, ComposerEvent, Draft, Outgoing, composer};
pub use image::{Image, ImageFailed, ImageKind, ImageLoaded, ImageProps, UNKNOWN_ASPECT, image, image_placeholder};
pub use lyrics::{
    ANCHOR_AT, DEPTH_OPACITY, Follow, Lit, LyricClicked, LyricLine, LyricWord, Lyrics, LyricsFollowChanged, LyricsKind,
    LyricsModel, LyricsProps, LyricsView, Row as LyricRow, RowKind as LyricRowKind, RowWord as LyricRowWord,
    SCALE_ANIM, SCROLL, SCROLL_CURVE, UNLIT_SCALE, anchor_y, bezier, depth_opacity, follow_offset, interlude_fill,
    lit_at, lyrics_view, rows as lyric_rows, word_fill,
};
pub use pipeline::{DecodeRequest, Decoded, ImageFit, ImagePipeline, ImageSource};
pub use shelf::{
    Shelf, ShelfArrows, ShelfHandle, ShelfKind, ShelfProps, ShelfScroll, ShelfScrolled, scroll_target, shelf,
    snap_offset,
};
pub use transport::{
    NowPlaying, PlayerBar, SEEK, SeekBar, SeekBarKind, SeekBarProps, SeekMetrics, SeekModel, SeekPhase, Seeked,
    TransportCommand, TransportLayout, player_bar, seek_bar, volume_icon,
};
pub use video::{Video, VideoEnded, VideoFailed, VideoKind, VideoProps, VideoReady, video};
