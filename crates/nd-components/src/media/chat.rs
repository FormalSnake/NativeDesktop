//! Conversation pieces: `chat_bubble` (a message bubble with its tail and reactions),
//! `avatar` and `avatar_group` (a photo or monogram in a circle, several people clustered in
//! one), and `chat_row`, which lays a message out the way Messages does.
//!
//! Neither macOS nor the GPUI platforms have a system bubble or avatar, so both are drawn from
//! theme tokens on every backend: sent bubbles in the accent with `on_accent` text, received
//! ones on `surface.raised_hover` (it still reads on a white window), monograms on the accent
//! container. Rules from messages' `thread_rows.rs`: a run is consecutive messages from one
//! sender; its bubbles tighten the corners they share ([`BUBBLE.tight`](BubbleMetrics::tight)),
//! only the run's last bubble has a tail, and runs are spaced wider than bubbles inside one.
//! One to three emoji alone are drawn large without a bubble.

use std::panic::Location;

use nd_ui::prelude::*;
use nd_ui::semantics::{Semantics, WidgetSemantics};
use nd_ui::{Key, Role, StyleRefinement};

use super::pipeline::ImageSource;
use crate::forms::{LabelTone, TextStyle, label, leaf};

/// Bubble geometry: the component's design, the same on every backend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BubbleMetrics {
    pub radius: f32,
    /// Corners a bubble shares with its neighbour in a run.
    pub tight: f32,
    pub padding_x: f32,
    pub padding_y: f32,
    /// The tail's lobe, outside the bubble's bottom corner.
    pub tail_width: f32,
    pub tail_height: f32,
    /// The widest a bubble grows, and its share of the row.
    pub max_width: f32,
    pub max_fraction: f32,
    /// A reaction pill's height and how far the pills rise above the bubble.
    pub reaction: f32,
    pub reaction_lift: f32,
    /// Space above a row that starts a run, and above one inside a run.
    pub run_gap: f32,
    pub in_run_gap: f32,
    /// Emoji-only messages: at most this many, drawn at `emoji_size`.
    pub emoji_max: usize,
    pub emoji_size: f32,
}

pub const BUBBLE: BubbleMetrics = BubbleMetrics {
    radius: 18.,
    tight: 5.,
    padding_x: 12.,
    padding_y: 7.,
    tail_width: 6.,
    tail_height: 14.,
    max_width: 460.,
    max_fraction: 0.62,
    reaction: 24.,
    reaction_lift: 14.,
    run_gap: 10.,
    in_run_gap: 2.,
    emoji_max: 3,
    emoji_size: 40.,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BubbleSide {
    /// The user's own: trailing, in the accent.
    #[default]
    Sent,
    Received,
}

/// Where a message sits in its sender's run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RunPosition {
    #[default]
    Single,
    First,
    Middle,
    Last,
}

impl RunPosition {
    pub fn starts_run(self) -> bool {
        matches!(self, RunPosition::Single | RunPosition::First)
    }

    pub fn ends_run(self) -> bool {
        matches!(self, RunPosition::Single | RunPosition::Last)
    }

    /// From whether the message joins the one before and the one after.
    pub fn of(joins_previous: bool, joins_next: bool) -> Self {
        match (joins_previous, joins_next) {
            (false, false) => RunPosition::Single,
            (false, true) => RunPosition::First,
            (true, true) => RunPosition::Middle,
            (true, false) => RunPosition::Last,
        }
    }
}

/// Corner radii `[top_left, top_right, bottom_right, bottom_left]` of a bubble at `position`.
pub fn bubble_corners(side: BubbleSide, position: RunPosition) -> [f32; 4] {
    let r = BUBBLE.radius;
    let t = BUBBLE.tight;
    let top = if matches!(position, RunPosition::Middle | RunPosition::Last) { t } else { r };
    let bottom = if matches!(position, RunPosition::First | RunPosition::Middle) { t } else { r };
    match side {
        BubbleSide::Sent => [r, top, bottom, r],
        BubbleSide::Received => [top, r, r, bottom],
    }
}

/// A reaction on a message (a tapback).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reaction {
    pub glyph: SharedString,
    pub count: u32,
    /// The user reacted with it.
    pub mine: bool,
}

/// Whether `text` is one to `max` emoji and nothing else.
pub fn is_emoji_only(text: &str, max: usize) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }
    let (mut count, mut joined, mut flag_halves) = (0, false, 0);
    for c in text.chars() {
        if c == '\u{200d}' {
            joined = true;
            continue;
        }
        let modifier = matches!(c, '\u{fe0f}' | '\u{fe0e}' | '\u{1f3fb}'..='\u{1f3ff}' | '\u{e0020}'..='\u{e007f}');
        if modifier || c.is_whitespace() {
            continue;
        }
        if !is_emoji(c) {
            return false;
        }
        if ('\u{1f1e6}'..='\u{1f1ff}').contains(&c) {
            // Two regional indicators are one flag.
            flag_halves += 1;
            if flag_halves % 2 == 1 {
                count += 1;
            }
        } else if !joined {
            count += 1;
        }
        joined = false;
    }
    count <= max
}

fn is_emoji(c: char) -> bool {
    matches!(c,
        '\u{1f300}'..='\u{1faff}'
        | '\u{2600}'..='\u{27bf}'
        | '\u{1f1e6}'..='\u{1f1ff}'
        | '\u{2b00}'..='\u{2bff}'
        | '\u{2300}'..='\u{23ff}'
        | '\u{3030}' | '\u{303d}' | '\u{3297}' | '\u{3299}' | '\u{a9}' | '\u{ae}' | '\u{2122}'
    )
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatBubbleProps {
    pub text: SharedString,
    pub side: BubbleSide,
    pub position: RunPosition,
    pub reactions: Vec<Reaction>,
    /// Drawn large without a bubble ([`is_emoji_only`] decides at build).
    pub emoji_only: bool,
    /// The text can be selected and copied.
    pub selectable: bool,
    pub enabled: bool,
    /// The time, shown on hover.
    pub tooltip: Option<SharedString>,
    pub style: StyleRefinement,
}

impl ChatBubbleProps {
    pub fn has_tail(&self) -> bool {
        self.position.ends_run() && !self.emoji_only
    }

    pub fn corners(&self) -> [f32; 4] {
        bubble_corners(self.side, self.position)
    }
}

/// Widget kind of `chat_bubble`.
pub struct ChatBubbleKind;

impl WidgetSemantics for ChatBubbleProps {
    fn semantics(&self) -> Semantics {
        let who = match self.side {
            BubbleSide::Sent => "Sent",
            BubbleSide::Received => "Received",
        };
        let mut value = who.to_owned();
        for r in &self.reactions {
            value.push_str(&format!(", {} {}", r.glyph, r.count));
        }
        Semantics::new(Role::Text, self.text.to_string()).value(value).enabled(self.enabled)
    }
}

pub struct ChatBubble(NativeElement<ChatBubbleProps>);

/// A message bubble. Its width follows the text up to [`BUBBLE.max_width`](BubbleMetrics).
#[track_caller]
pub fn chat_bubble(text: impl Into<SharedString>, side: BubbleSide) -> ChatBubble {
    let text = text.into();
    let props = ChatBubbleProps {
        emoji_only: is_emoji_only(&text, BUBBLE.emoji_max),
        text,
        side,
        position: RunPosition::Single,
        reactions: Vec::new(),
        selectable: true,
        enabled: true,
        tooltip: None,
        style: StyleRefinement::default(),
    };
    ChatBubble(NativeElement::described::<ChatBubbleKind>(Key::Site(Location::caller()), props))
}

impl ChatBubble {
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        *self.0.key_mut() = Key::Id(id.into());
        self
    }

    pub fn position(mut self, position: RunPosition) -> Self {
        self.0.props_mut().position = position;
        self
    }

    pub fn reactions(mut self, reactions: impl IntoIterator<Item = Reaction>) -> Self {
        self.0.props_mut().reactions = reactions.into_iter().take(3).collect();
        self
    }

    /// The time, shown on hover.
    pub fn time(mut self, time: impl Into<SharedString>) -> Self {
        self.0.props_mut().tooltip = Some(time.into());
        self
    }
}

leaf!(ChatBubble, ChatBubbleProps);

/// Someone an avatar shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    pub name: SharedString,
    pub image: Option<ImageSource>,
}

impl Person {
    pub fn new(name: impl Into<SharedString>) -> Self {
        Self { name: name.into(), image: None }
    }

    pub fn image(mut self, image: impl Into<ImageSource>) -> Self {
        self.image = Some(image.into());
        self
    }
}

/// The monogram for `name`: the first letters of its first and last words, uppercased, or
/// `#` for a name that is empty or a phone number (drawn as a person glyph).
pub fn initials(name: &str) -> String {
    let name = name.trim();
    match name.chars().next() {
        None => return "#".into(),
        Some(c) if c.is_ascii_digit() || c == '+' || c == '(' => return "#".into(),
        _ => {}
    }
    let words: Vec<&str> = name.split_whitespace().collect();
    let first = words.first().and_then(|w| w.chars().next());
    let last = (words.len() > 1).then(|| words.last().and_then(|w| w.chars().next())).flatten();
    first.into_iter().chain(last).flat_map(char::to_uppercase).collect()
}

/// A monogram's text size for an avatar `size` across; below [`MONOGRAM_MIN`] it shows none.
pub fn monogram_size(size: f32) -> f32 {
    (size * 0.38).round()
}

pub const MONOGRAM_MIN: f32 = 18.;

/// Where each person of a group sits inside a `size` circle: up to four mini avatars
/// [`AVATAR_GROUP_SCALE`] of it, one row for two, one over two for three, two by two for four.
pub fn group_layout(count: usize, size: f32) -> Vec<nd_core::Rect> {
    let mini = (size * AVATAR_GROUP_SCALE).round();
    let gap = 1.;
    let rows: &[usize] = match count.min(4) {
        0 => &[],
        1 => &[1],
        2 => &[2],
        3 => &[1, 2],
        _ => &[2, 2],
    };
    let height = rows.len() as f32 * mini + (rows.len().saturating_sub(1)) as f32 * gap;
    let mut y = ((size - height) / 2.).round();
    let mut out = Vec::new();
    for &n in rows {
        let width = n as f32 * mini + (n - 1) as f32 * gap;
        let mut x = ((size - width) / 2.).round();
        for _ in 0..n {
            out.push(nd_core::Rect::new(x, y, mini, mini));
            x += mini + gap;
        }
        y += mini + gap;
    }
    out
}

pub const AVATAR_GROUP_SCALE: f32 = 0.42;

#[derive(Clone, Debug, PartialEq)]
pub struct AvatarProps {
    /// One person, or up to four of a group clustered in one circle.
    pub people: Vec<Person>,
    /// Diameter in points.
    pub size: f32,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    pub style: StyleRefinement,
}

/// Widget kind of `avatar` and `avatar_group`.
pub struct AvatarKind;

impl WidgetSemantics for AvatarProps {
    fn semantics(&self) -> Semantics {
        let names: Vec<&str> = self.people.iter().map(|p| p.name.as_ref()).collect();
        let name = match names.as_slice() {
            [] => String::new(),
            [one] => (*one).to_owned(),
            [a, b] => format!("{a} and {b}"),
            [a, b, rest @ ..] => format!("{a}, {b} and {} more", rest.len()),
        };
        Semantics::new(Role::Image, name).enabled(self.enabled)
    }
}

pub struct Avatar(NativeElement<AvatarProps>);

fn avatar_of(people: Vec<Person>, size: f32, site: &'static Location<'static>) -> Avatar {
    let props = AvatarProps { people, size, enabled: true, tooltip: None, style: StyleRefinement::default() };
    let el = NativeElement::described::<AvatarKind>(Key::Site(site), props).size(px(size)).flex_none();
    Avatar(el)
}

/// A person's photo, or their initials, in a circle `size` points across.
#[track_caller]
pub fn avatar(person: Person, size: f32) -> Avatar {
    avatar_of(vec![person], size, Location::caller())
}

/// A group's avatar: its first four people clustered in one circle.
#[track_caller]
pub fn avatar_group(people: impl IntoIterator<Item = Person>, size: f32) -> Avatar {
    avatar_of(people.into_iter().collect(), size, Location::caller())
}

impl Avatar {
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        *self.0.key_mut() = Key::Id(id.into());
        self
    }
}

leaf!(Avatar, AvatarProps);

/// One message as a [`chat_row`] lays it out.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatMessage {
    pub text: SharedString,
    pub side: BubbleSide,
    pub position: RunPosition,
    /// Who sent a received message in a group: their avatar shows beside the run's last
    /// bubble and their name above its first.
    pub sender: Option<Person>,
    pub time: Option<SharedString>,
    pub reactions: Vec<Reaction>,
    /// Under the user's last message: "Delivered", "Read 3:07 PM".
    pub receipt: Option<SharedString>,
}

impl ChatMessage {
    pub fn sent(text: impl Into<SharedString>) -> Self {
        Self::new(text, BubbleSide::Sent)
    }

    pub fn received(text: impl Into<SharedString>) -> Self {
        Self::new(text, BubbleSide::Received)
    }

    fn new(text: impl Into<SharedString>, side: BubbleSide) -> Self {
        Self {
            text: text.into(),
            side,
            position: RunPosition::Single,
            sender: None,
            time: None,
            reactions: Vec::new(),
            receipt: None,
        }
    }

    pub fn position(mut self, position: RunPosition) -> Self {
        self.position = position;
        self
    }

    pub fn sender(mut self, sender: Person) -> Self {
        self.sender = Some(sender);
        self
    }

    pub fn time(mut self, time: impl Into<SharedString>) -> Self {
        self.time = Some(time.into());
        self
    }

    pub fn reactions(mut self, reactions: impl IntoIterator<Item = Reaction>) -> Self {
        self.reactions = reactions.into_iter().collect();
        self
    }

    pub fn receipt(mut self, receipt: impl Into<SharedString>) -> Self {
        self.receipt = Some(receipt.into());
        self
    }
}

/// The avatar column beside received bubbles in a group thread.
pub const AVATAR_COLUMN: f32 = 28.;

/// A message laid out as a thread row: aligned to its side, spaced by its run, the sender's
/// avatar and name in a group, a receipt under it. `id` keys the row's bubble.
pub fn chat_row(id: impl Into<SharedString>, message: ChatMessage) -> AnyElement {
    let id: SharedString = id.into();
    let sent = message.side == BubbleSide::Sent;
    let gap = if message.position.starts_run() { BUBBLE.run_gap } else { BUBBLE.in_run_gap };
    let lift = if message.reactions.is_empty() { 0. } else { BUBBLE.reaction_lift };
    let mut bubble = chat_bubble(message.text.clone(), message.side)
        .id(id.clone())
        .position(message.position)
        .reactions(message.reactions.clone());
    if let Some(time) = message.time.clone() {
        bubble = bubble.time(time);
    }
    let group = !sent && message.sender.is_some();
    let avatar_slot = group.then(|| match (&message.sender, message.position.ends_run()) {
        (Some(person), true) => avatar(person.clone(), AVATAR_COLUMN).id(format!("{id}.avatar")).into_any_element(),
        _ => div().w(px(AVATAR_COLUMN)).flex_none().into_any_element(),
    });
    let name = (group && message.position.starts_run())
        .then(|| message.sender.as_ref().map(|p| p.name.clone()))
        .flatten()
        .map(|n| {
            label(n).text_style(TextStyle::Caption2).tone(LabelTone::Secondary).ml(px(if group {
                AVATAR_COLUMN + 8. + BUBBLE.padding_x
            } else {
                BUBBLE.padding_x
            }))
        });
    let line = h_flex()
        .w_full()
        .items_end()
        .gap_2()
        .when(sent, |row| row.justify_end())
        .children(avatar_slot)
        .child(bubble.mt(px(lift)).max_w(relative(BUBBLE.max_fraction)));
    let receipt = message.receipt.clone().map(|r| {
        label(r).text_style(TextStyle::Caption2).tone(LabelTone::Secondary).when(sent, |l| l.mr(px(BUBBLE.padding_x)))
    });
    v_flex()
        .w_full()
        .pt(px(gap))
        .when(sent, |col| col.items_end())
        .children(name)
        .child(line)
        .children(receipt)
        .into_any_element()
}
