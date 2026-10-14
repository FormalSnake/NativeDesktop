//! The Media page: images from the pipeline, a player bar in both layouts, synced lyrics on
//! the same clock, a shelf of cards, a chat thread in a virtual list with the composer under it,
//! and avatars.

use std::io::Cursor;
use std::time::Duration;

use nd::prelude::*;

/// A generated cover: a diagonal blend between two colours, so the gallery needs no files.
fn cover(key: &str, size: u32, from: [u8; 3], to: [u8; 3]) -> ImageSource {
    let img = image::RgbaImage::from_fn(size, size, |x, y| {
        let t = (x + y) as f32 / (2 * size) as f32;
        let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t) as u8;
        image::Rgba([mix(from[0], to[0]), mix(from[1], to[1]), mix(from[2], to[2]), 255])
    });
    let mut bytes = Vec::new();
    img.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png).expect("png encodes");
    ImageSource::bytes(key.to_owned(), bytes)
}

/// A small animated GIF: a dot circling a square.
fn spinner_gif() -> ImageSource {
    use image::codecs::gif::{GifEncoder, Repeat};
    let mut bytes = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut bytes);
        enc.set_repeat(Repeat::Infinite).expect("repeat");
        for i in 0..12u32 {
            let angle = i as f32 / 12. * std::f32::consts::TAU;
            let (cx, cy) = (32. + 18. * angle.cos(), 32. + 18. * angle.sin());
            let img = image::RgbaImage::from_fn(64, 64, |x, y| {
                let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                if d < 7. { image::Rgba([255, 255, 255, 255]) } else { image::Rgba([90, 60, 200, 255]) }
            });
            let frame = image::Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(80, 1));
            enc.encode_frame(frame).expect("frame");
        }
    }
    ImageSource::bytes("gallery-spinner", bytes)
}

const SONG: [(u64, &str); 14] = [
    (6, "Streetlights hum a quiet tune"),
    (10, "We trace the river under the moon"),
    (14, "Every window holds a light"),
    (18, "Every light a story for the night"),
    (26, "Hold on, the morning's far away"),
    (30, "Hold on, there's nothing more to say"),
    (34, "Turn the radio up and drive"),
    (38, "The city sings and we're alive"),
    (50, "Paper planes across the bay"),
    (54, "Carry all our words away"),
    (58, "If the stars forget our names"),
    (62, "We will write them all again"),
    (70, "Hold on, the morning's far away"),
    (74, "Hold on, we're halfway home today"),
];

fn lyrics() -> Lyrics {
    Lyrics::synced(SONG.iter().enumerate().map(|(i, (at, text))| {
        let end = SONG.get(i + 1).map_or(at + 4, |n| (n.0).min(at + 4));
        LyricLine::new(Duration::from_secs(*at), *text).end(Duration::from_secs(end))
    }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Who {
    Me,
    Ada,
    Grace,
}

struct Message {
    who: Who,
    text: String,
    time: &'static str,
}

/// The length the page shows without audio, for still goldens.
const DURATION: Duration = Duration::from_secs(243);
/// The generated tone's length: long enough for the lyrics to play through.
const TONE: Duration = Duration::from_secs(80);
const TONE_RATE: u32 = 22_050;

/// A quiet arpeggio as 16-bit mono PCM WAV, so the gallery plays real audio without shipping a
/// file. One note every half second, each with a short attack and a long release.
fn tone_wav() -> Vec<u8> {
    const NOTES: [f32; 8] = [261.63, 329.63, 392.0, 523.25, 392.0, 329.63, 293.66, 349.23];
    let samples = (TONE.as_secs_f32() * TONE_RATE as f32) as usize;
    let note_len = TONE_RATE as usize / 2;
    let mut pcm = Vec::with_capacity(samples * 2);
    for i in 0..samples {
        let note = NOTES[(i / note_len) % NOTES.len()];
        let t = (i % note_len) as f32 / TONE_RATE as f32;
        let envelope = (t / 0.01).min(1.) * (-t * 5.).exp();
        let v = (2. * std::f32::consts::PI * note * i as f32 / TONE_RATE as f32).sin() * envelope * 0.25;
        pcm.extend_from_slice(&((v * i16::MAX as f32) as i16).to_le_bytes());
    }
    let mut wav = Vec::with_capacity(44 + pcm.len());
    let le32 = |v: u32| v.to_le_bytes();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&le32(36 + pcm.len() as u32));
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&le32(16));
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&le32(TONE_RATE));
    wav.extend_from_slice(&le32(TONE_RATE * 2));
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&le32(pcm.len() as u32));
    wav.extend_from_slice(&pcm);
    wav
}

/// Drives a system call whose answer the page does not need.
fn run(call: impl std::future::Future<Output = nd::system::Result<()>> + Send + 'static) {
    nd::system::spawn(call, |_, _| {});
}

/// The tone in the app's cache directory, written on first use.
fn tone_file() -> Option<std::path::PathBuf> {
    let path = nd::system::app_dirs().ensure_cache().ok()?.join("gallery-tone.wav");
    if !path.exists() {
        std::fs::write(&path, tone_wav()).ok()?;
    }
    Some(path)
}

pub struct MediaPage {
    clock: PlaybackClock,
    duration: Duration,
    /// The tone, when the page plays real audio.
    tone: Option<std::path::PathBuf>,
    player: Option<nd::system::Player>,
    starting: bool,
    /// State events the player sent, the last one's clock.
    reports: Vec<PlaybackClock>,
    volume: f32,
    muted: bool,
    lyrics: std::sync::Arc<Lyrics>,
    art: ImageSource,
    covers: Vec<(ImageSource, &'static str, &'static str)>,
    spinner: ImageSource,
    shelf: ShelfHandle,
    thread: ListState<u32>,
    messages: Vec<Message>,
    draft: Draft,
    last: SharedString,
}

impl MediaPage {
    pub fn new(audio: bool, cx: &mut Context<Self>) -> Self {
        let tone = audio.then(tone_file).flatten();
        if tone.is_some() {
            let page = cx.weak_entity();
            nd::system::on_event(&nd::system::system(), move |event, app| {
                page.update(app, |this, cx| {
                    this.system_event(event, cx);
                    cx.notify();
                });
            });
        }
        let palette: [([u8; 3], [u8; 3], &str, &str); 8] = [
            ([255, 94, 98], [255, 195, 113], "Sunset Drive", "The Lanterns"),
            ([58, 123, 213], [58, 96, 115], "Harbour Lights", "Northbound"),
            ([17, 153, 142], [56, 239, 125], "Green Room", "Field Notes"),
            ([142, 45, 226], [74, 0, 224], "Violet Hour", "Mira Sol"),
            ([252, 70, 107], [63, 94, 251], "Neon Rain", "Static Bloom"),
            ([255, 175, 189], [255, 195, 160], "Peach Fuzz", "June Avenue"),
            ([33, 147, 176], [109, 213, 237], "Low Tide", "Coastline"),
            ([238, 156, 167], [255, 221, 225], "Paper Hearts", "The Kites"),
        ];
        let covers = palette
            .iter()
            .enumerate()
            .map(|(i, (a, b, title, artist))| (cover(&format!("gallery-cover-{i}"), 256, *a, *b), *title, *artist))
            .collect();
        let messages = vec![
            Message { who: Who::Ada, text: "Did you hear the new album?".into(), time: "9:41 AM" },
            Message { who: Who::Ada, text: "The second track is unreal".into(), time: "9:41 AM" },
            Message { who: Who::Grace, text: "Playing it now".into(), time: "9:42 AM" },
            Message { who: Who::Me, text: "Same, on repeat since this morning".into(), time: "9:43 AM" },
            Message { who: Who::Me, text: "\u{1f3a7}\u{1f525}".into(), time: "9:43 AM" },
        ];
        let mut thread = ListState::new(cx).follow(Follow::Tail).estimated_row_height(px(44.));
        thread.set_items_versioned((0..messages.len() as u32).map(|i| (i, 0)), cx);
        Self {
            clock: PlaybackClock::paused(if tone.is_some() { Duration::ZERO } else { Duration::from_secs(36) }),
            duration: if tone.is_some() { TONE } else { DURATION },
            tone,
            player: None,
            starting: false,
            reports: Vec::new(),
            volume: 0.6,
            muted: false,
            lyrics: std::sync::Arc::new(lyrics()),
            art: cover("gallery-art", 256, [255, 94, 98], [142, 45, 226]),
            covers,
            spinner: spinner_gif(),
            shelf: ShelfHandle::new(),
            thread,
            messages,
            draft: Draft::default(),
            last: "Nothing yet".into(),
        }
    }

    /// The clocks the OS player reported, oldest first.
    pub fn reports(&self) -> &[PlaybackClock] {
        &self.reports
    }

    pub fn last(&self) -> &SharedString {
        &self.last
    }

    fn command(&mut self, command: &TransportCommand, cx: &mut Context<Self>) {
        let now = self.clock.now(Some(self.duration));
        match *command {
            TransportCommand::TogglePlay => {
                let playing = !self.clock.playing;
                self.clock = if playing { PlaybackClock::playing(now) } else { PlaybackClock::paused(now) };
                match (self.started_player(), playing) {
                    (Some(p), true) => run(p.resume()),
                    (Some(p), false) => run(p.pause()),
                    (None, true) => self.start(now, cx),
                    (None, false) => {}
                }
            }
            TransportCommand::Seek(p) => {
                self.clock = if self.clock.playing { PlaybackClock::playing(p) } else { PlaybackClock::paused(p) };
                if let Some(player) = self.started_player() {
                    run(player.seek(p));
                }
            }
            TransportCommand::Previous => {
                self.clock = PlaybackClock::paused(Duration::ZERO);
                if let Some(player) = self.started_player() {
                    run(player.pause());
                    run(player.seek(Duration::ZERO));
                }
            }
            TransportCommand::SetVolume(v) => {
                self.volume = v;
                self.muted = false;
            }
            TransportCommand::ToggleMute => self.muted = !self.muted,
            TransportCommand::Next | TransportCommand::Scrub(_) => {}
        }
        if let Some(player) = self.started_player() {
            run(player.set_volume(if self.muted { 0. } else { self.volume }));
        }
        self.publish();
        self.last = format!("{command:?}").into();
    }

    /// The player once the OS has started it; controls sent before that would find no sound.
    fn started_player(&self) -> Option<&nd::system::Player> {
        self.player.as_ref().filter(|_| !self.starting)
    }

    /// Starts the tone at `at`; the OS reports it started on a later turn.
    fn start(&mut self, at: Duration, cx: &mut Context<Self>) {
        let Some(tone) = self.tone.clone() else { return };
        if self.starting {
            return;
        }
        self.starting = true;
        let volume = if self.muted { 0. } else { self.volume };
        let options = nd::system::PlayOptions::path(tone).volume(volume);
        let (player, started) = nd::system::system().audio().play(options);
        self.player = Some(player.clone());
        let page = cx.weak_entity();
        nd::system::spawn(started, move |started, app| {
            page.update(app, |this, cx| {
                this.starting = false;
                match started {
                    Ok(()) => {
                        if !at.is_zero() {
                            run(player.seek(at));
                        }
                        if !this.clock.playing {
                            run(player.pause());
                        }
                    }
                    Err(e) => {
                        this.player = None;
                        this.clock = PlaybackClock::paused(at);
                        this.last = format!("Could not play: {e}").into();
                    }
                }
                cx.notify();
            });
        });
    }

    /// The player's own state and the OS media keys.
    fn system_event(&mut self, event: &nd::system::SystemEvent, cx: &mut Context<Self>) {
        match event {
            nd::system::SystemEvent::Audio { player, event }
                if self.player.as_ref().is_some_and(|p| p.id() == *player) =>
            {
                if let Some((clock, duration)) = nd::media::clock_from(event) {
                    self.clock = clock;
                    self.reports.push(clock);
                    if let Some(d) = duration {
                        self.duration = d;
                    }
                    self.publish();
                }
            }
            nd::system::SystemEvent::Media(command) => {
                let now = self.clock.now(Some(self.duration));
                if let Some(command) = nd::media::command_from(*command, self.clock.playing, now) {
                    self.command(&command, cx);
                }
            }
            _ => {}
        }
    }

    /// What the OS shows as Now Playing.
    fn publish(&self) {
        if self.tone.is_some() {
            run(nd::system::system().media().set(nd::media::now_playing(&self.now_playing(), None)));
        }
    }

    fn now_playing(&self) -> NowPlaying {
        NowPlaying::new("Harbour Lights")
            .subtitle("Northbound \u{00b7} Night Drive")
            .artwork(self.art.clone())
            .clock(self.clock)
            .duration(self.duration)
            .buffered(if self.tone.is_some() { self.duration } else { Duration::from_secs(150) })
            .volume(self.volume, self.muted)
    }

    fn person(who: Who) -> Option<Person> {
        match who {
            Who::Me => None,
            Who::Ada => Some(Person::new("Ada Lovelace")),
            Who::Grace => Some(Person::new("Grace Hopper")),
        }
    }

    fn row(&mut self, index: u32, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        let i = index as usize;
        let Some(m) = self.messages.get(i) else { return div().into_any_element() };
        let joins = |other: Option<&Message>| other.is_some_and(|o| o.who == m.who);
        let position = RunPosition::of(
            joins(i.checked_sub(1).and_then(|p| self.messages.get(p))),
            joins(self.messages.get(i + 1)),
        );
        let mut message = match Self::person(m.who) {
            None => ChatMessage::sent(m.text.clone()),
            Some(p) => ChatMessage::received(m.text.clone()).sender(p),
        };
        message = message.position(position).time(m.time);
        if i == 1 {
            message = message.reactions([Reaction { glyph: "\u{2764}\u{fe0f}".into(), count: 2, mine: true }]);
        }
        if m.who == Who::Me && i + 1 == self.messages.len() {
            message = message.receipt("Delivered");
        }
        div().px(px(12.)).child(chat_row(format!("msg-{i}"), message)).into_any_element()
    }

    fn send(&mut self, out: Outgoing, cx: &mut Context<Self>) {
        if let Some(text) = out.text {
            self.messages.push(Message { who: Who::Me, text: text.to_string(), time: "Now" });
        }
        for a in out.attachments {
            self.messages.push(Message { who: Who::Me, text: format!("\u{1f4ce} {}", a.name), time: "Now" });
        }
        let n = self.messages.len() as u32;
        self.thread.set_items_versioned((0..n).map(|i| (i, 0)), cx);
        self.last = "Sent a message".into();
    }
}

fn heading(title: &'static str) -> impl IntoElement {
    label(title).text_style(TextStyle::Title3)
}

impl Render for MediaPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cards = self.covers.iter().enumerate().map(|(i, (art, title, artist))| {
            v_flex()
                .id(format!("card-{i}"))
                .w(px(132.))
                .flex_none()
                .gap_1()
                .child(image(art.clone()).id(format!("card-art-{i}")).alt(*title).size(px(132.)).rounded_lg())
                .child(label(*title).text_style(TextStyle::Headline))
                .child(label(*artist).text_style(TextStyle::Subheadline).secondary())
        });
        v_flex()
            .id("media-page")
            .size_full()
            .overflow_y_scroll()
            .gap_4()
            .pb(px(24.))
            .child(heading("Images"))
            .child(
                h_flex()
                    .gap_4()
                    .items_end()
                    .child(image(self.art.clone()).id("img-rounded").alt("Album art").size(px(96.)).rounded_lg())
                    .child(image(self.covers[3].0.clone()).id("img-circle").alt("Artist").size(px(64.)).rounded_full())
                    .child(
                        image(self.covers[1].0.clone())
                            .id("img-contain")
                            .alt("Letterboxed")
                            .contain()
                            .w(px(140.))
                            .h(px(80.))
                            .rounded_md(),
                    )
                    .child(image(self.spinner.clone()).id("img-animated").alt("Animated").size(px(64.)).rounded_md())
                    .child(image_placeholder().id("img-placeholder").alt("Loading").size(px(64.)).rounded_md()),
            )
            .child(heading("Player"))
            .child(player_bar("player", self.now_playing()).on_command(cx.listener(
                |this, c: &TransportCommand, _, cx| {
                    this.command(c, cx);
                    cx.notify();
                },
            )))
            .child(div().w(px(320.)).child(player_bar("mini", self.now_playing()).compact().on_command(cx.listener(
                |this, c: &TransportCommand, _, cx| {
                    this.command(c, cx);
                    cx.notify();
                },
            ))))
            .child(heading("Lyrics"))
            .child(
                lyrics_view("lyrics", self.lyrics.clone(), self.clock)
                    .duration(Some(self.duration))
                    .resync_after(Duration::from_secs(8))
                    .on_line_click(cx.listener(|this, e: &LyricClicked, _, cx| {
                        this.command(&TransportCommand::Seek(e.start), cx);
                        cx.notify();
                    }))
                    .w_full()
                    .h(px(220.))
                    .flex_none(),
            )
            .child(heading("Shelf"))
            .child(shelf("shelf", &self.shelf).label("Recently played").gap_4().h(px(190.)).flex_none().children(cards))
            .child(heading("Conversation"))
            .child(
                h_flex()
                    .gap_3()
                    .child(avatar(Person::new("Ada Lovelace").image(self.covers[2].0.clone()), 44.).id("avatar-photo"))
                    .child(avatar(Person::new("Grace Hopper"), 44.).id("avatar-initials"))
                    .child(
                        avatar_group(
                            [
                                Person::new("Ada Lovelace"),
                                Person::new("Grace Hopper"),
                                Person::new("Alan Turing"),
                                Person::new("Me"),
                            ],
                            44.,
                        )
                        .id("avatar-group"),
                    ),
            )
            .child(
                list("thread", &self.thread, cx.processor(Self::row))
                    .estimated_row_height(px(44.))
                    .w_full()
                    .h(px(260.))
                    .flex_none(),
            )
            .child(composer("composer", &self.draft).placeholder("iMessage").on_event(cx.listener(
                |this, e: &ComposerEvent, _, cx| {
                    if *e == ComposerEvent::Attach {
                        this.draft.attach("/System/Library/Desktop Pictures/Sequoia.heic");
                    }
                    if let Some(out) = this.draft.apply(e) {
                        this.send(out, cx);
                    }
                    cx.notify();
                },
            )))
    }
}
