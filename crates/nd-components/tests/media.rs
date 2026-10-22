//! Tier 1 behaviour of the media family on `TestBackend`: the image pipeline (display-size
//! decode, header probe, animation, caches), the playback clock and seek model, the player bar,
//! synced lyrics, shelves, chat bubbles and avatars, and the composer.

mod common;

use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use common::*;
use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, ImageFormat, Rgba, RgbaImage};
use nd_components::forms::{ImagesPasted, PastedImage, SliderChanged, TextAreaProps, TextSubmitted};
use nd_components::media::pipeline::{EDGE_STEP, PipelineConfig, decode};
use nd_components::*;
use nd_core::time::Instant;
use nd_ui::prelude::*;
use nd_ui::semantics::SemanticAction;
use nd_ui::{ClickEvent, Rect, Role};

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = RgbaImage::from_pixel(w, h, Rgba([200, 40, 40, 255]));
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).unwrap();
    out
}

fn gif(w: u32, h: u32, frames: usize, delay_ms: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        enc.set_repeat(Repeat::Infinite).unwrap();
        for i in 0..frames {
            let shade = (i * 40 % 255) as u8;
            let img = RgbaImage::from_pixel(w, h, Rgba([shade, 0, 0, 255]));
            enc.encode_frame(Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms(delay_ms, 1))).unwrap();
        }
    }
    out
}

fn pipeline(fetch: impl Fn(&str) -> Result<Vec<u8>, String> + Send + Sync + 'static) -> ImagePipeline {
    ImagePipeline::new(PipelineConfig {
        memory_budget: 64 << 20,
        animation_budget: 48 << 20,
        disk_dir: None,
        workers: 2,
        fetch: Arc::new(fetch),
    })
}

fn load(p: &ImagePipeline, request: DecodeRequest) -> Result<Arc<Decoded>, String> {
    let (tx, rx) = mpsc::channel();
    p.load(request, move |r| {
        let _ = tx.send(r);
    });
    rx.recv_timeout(Duration::from_secs(10)).expect("the pipeline answers")
}

#[test]
fn requests_round_the_box_up_to_the_edge_step_at_the_scale() {
    let r = DecodeRequest::at(ImageSource::from("a.png"), 100., 30., 2., ImageFit::Cover);
    assert_eq!((r.width, r.height), (256, 64));
    assert_eq!(r.width % EDGE_STEP, 0);
    let open = DecodeRequest::at(ImageSource::from("a.png"), 0., 0., 2., ImageFit::Cover);
    assert_eq!((open.width, open.height), (0, 0), "an unsized box decodes full size");
}

#[test]
fn decode_scales_down_to_the_box_and_never_up() {
    let bytes = png(800, 400);
    let source = ImageSource::bytes("wide", bytes.clone());
    let cover = decode(
        &bytes,
        &DecodeRequest { source: source.clone(), width: 128, height: 128, fit: ImageFit::Cover, tint: None },
        1 << 30,
    )
    .unwrap();
    assert_eq!((cover.width, cover.height), (256, 128), "cover fills the shorter side");
    let contain = decode(
        &bytes,
        &DecodeRequest { source: source.clone(), width: 128, height: 128, fit: ImageFit::Contain, tint: None },
        1 << 30,
    )
    .unwrap();
    assert_eq!((contain.width, contain.height), (128, 64), "contain fits the longer side");
    let big =
        decode(&bytes, &DecodeRequest { source, width: 4096, height: 4096, fit: ImageFit::Cover, tint: None }, 1 << 30)
            .unwrap();
    assert_eq!((big.width, big.height), (800, 400), "never scaled up");
    assert_eq!((big.source_width, big.source_height), (800, 400));
    assert_eq!(big.frames.len(), 1);
    assert_eq!(big.frames[0].rgba.len(), 800 * 400 * 4);
}

#[test]
fn animated_gifs_keep_every_frame_and_slow_down_too_fast_delays() {
    let bytes = gif(16, 16, 4, 10);
    let request = DecodeRequest {
        source: ImageSource::bytes("gif", bytes.clone()),
        width: 0,
        height: 0,
        fit: ImageFit::Cover,
        tint: None,
    };
    let d = decode(&bytes, &request, 1 << 30).unwrap();
    assert!(d.is_animated());
    assert_eq!(d.frames.len(), 4);
    assert!(d.frames.iter().all(|f| f.delay == Duration::from_millis(100)), "a 10 ms delay plays at 100 ms");
    assert_eq!(d.loop_duration(), Duration::from_millis(400));
    assert_eq!(d.frame_at(Duration::from_millis(250)), 2);
    assert_eq!(d.frame_at(Duration::from_millis(450)), 0, "it loops");
}

#[test]
fn an_animation_over_budget_drops_frames_and_keeps_its_length() {
    let bytes = gif(16, 16, 8, 50);
    let request = DecodeRequest {
        source: ImageSource::bytes("gif8", bytes.clone()),
        width: 0,
        height: 0,
        fit: ImageFit::Cover,
        tint: None,
    };
    let one_frame = 16 * 16 * 4;
    let d = decode(&bytes, &request, one_frame * 3).unwrap();
    assert_eq!(d.frames.len(), 2, "8 frames thinned by 4 to fit 3 frames' budget");
    assert_eq!(d.loop_duration(), Duration::from_millis(400), "dropped delays fold into kept frames");
}

#[test]
fn the_header_gives_the_size_without_decoding() {
    let p = pipeline(|_| Err("offline".into()));
    let source = ImageSource::bytes("hdr", png(640, 480));
    assert_eq!(p.intrinsic_size(&source), Some((640, 480)));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.png");
    std::fs::write(&path, png(30, 90)).unwrap();
    assert_eq!(p.intrinsic_size(&ImageSource::from(path)), Some((30, 90)));
    assert_eq!(p.intrinsic_size(&ImageSource::url("https://example.com/x.png")), None, "not downloaded yet");
}

#[test]
fn loads_for_one_request_share_a_decode_and_hit_memory_after() {
    let fetches = Arc::new(AtomicUsize::new(0));
    let counted = fetches.clone();
    let p = pipeline(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(50));
        Ok(png(64, 64))
    });
    let request = DecodeRequest::at(ImageSource::url("https://example.com/a.png"), 32., 32., 2., ImageFit::Cover);
    let (tx, rx) = mpsc::channel();
    for _ in 0..3 {
        let tx = tx.clone();
        p.load(request.clone(), move |r| tx.send(r.map(|d| d.width)).unwrap());
    }
    for _ in 0..3 {
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), Ok(64));
    }
    assert_eq!(fetches.load(Ordering::SeqCst), 1, "three loads, one fetch");
    assert!(p.get(&request).is_some(), "now in memory");
    assert_eq!(p.intrinsic_size(&request.source), Some((64, 64)), "the size is known once decoded");
    assert!(p.memory_used() >= 64 * 64 * 4);
}

#[test]
fn downloads_land_on_disk_and_a_new_process_reads_them_back() {
    let dir = tempfile::tempdir().unwrap();
    let fetches = Arc::new(AtomicUsize::new(0));
    let make = |fetches: Arc<AtomicUsize>| {
        ImagePipeline::new(PipelineConfig {
            disk_dir: Some(dir.path().to_owned()),
            workers: 1,
            fetch: Arc::new(move |_| {
                fetches.fetch_add(1, Ordering::SeqCst);
                Ok(png(20, 10))
            }),
            ..PipelineConfig::default()
        })
    };
    let url = ImageSource::url("https://example.com/art.png");
    let request = DecodeRequest { source: url.clone(), width: 0, height: 0, fit: ImageFit::Cover, tint: None };
    load(&make(fetches.clone()), request.clone()).unwrap();
    let second = make(fetches.clone());
    assert_eq!(second.intrinsic_size(&url), Some((20, 10)), "the header reads from the disk cache");
    load(&second, request).unwrap();
    assert_eq!(fetches.load(Ordering::SeqCst), 1, "the second pipeline never fetched");
    let names: Vec<String> =
        std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    assert_eq!(names.len(), 1);
    assert!(!names[0].ends_with(".tmp"), "no temp file is left behind");
}

#[test]
fn a_failure_is_remembered_instead_of_retried_at_once() {
    let fetches = Arc::new(AtomicUsize::new(0));
    let counted = fetches.clone();
    let p = pipeline(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
        Err("404".into())
    });
    let request = DecodeRequest {
        source: ImageSource::url("https://example.com/gone"),
        width: 0,
        height: 0,
        fit: ImageFit::Cover,
        tint: None,
    };
    assert_eq!(load(&p, request.clone()).unwrap_err(), "404");
    assert_eq!(load(&p, request.clone()).unwrap_err(), "404");
    assert_eq!(fetches.load(Ordering::SeqCst), 1);
    assert_eq!(p.failed(&request).as_deref(), Some("404"));
}

#[test]
fn memory_evicts_least_recent_but_never_what_was_just_shown() {
    let p = ImagePipeline::new(PipelineConfig {
        memory_budget: 2 * 32 * 32 * 4,
        disk_dir: None,
        workers: 1,
        fetch: Arc::new(|_| Err("no".into())),
        ..PipelineConfig::default()
    });
    let req = |k: &str| DecodeRequest {
        source: ImageSource::bytes(k.to_owned(), png(32, 32)),
        width: 0,
        height: 0,
        fit: ImageFit::Cover,
        tint: None,
    };
    load(&p, req("a")).unwrap();
    load(&p, req("b")).unwrap();
    load(&p, req("c")).unwrap();
    assert!(p.get(&req("a")).is_some(), "everything was used inside the last second: over budget, not evicted");
    std::thread::sleep(Duration::from_millis(1100));
    p.touch(&req("c"));
    load(&p, req("d")).unwrap();
    assert!(p.get(&req("c")).is_some(), "touched just now");
    assert!(p.get(&req("d")).is_some(), "the newest");
    assert!(p.memory_used() <= 2 * 32 * 32 * 4, "back under budget");
}

#[derive(Default)]
struct Pics {
    loaded: Vec<ImageLoaded>,
}

#[test]
fn image_describes_itself_and_sizes_its_placeholder_from_the_hint() {
    let mut rig = Rig::new(Pics::default(), |_, cx| {
        v_flex()
            .child(image("missing.png").id("art").alt("Album art").size_hint(1200, 600).on_load(cx.listener(
                |h, e: &ImageLoaded, _, cx| {
                    h.s.loaded.push(*e);
                    cx.notify();
                },
            )))
            .into_any_element()
    });
    assert!(rig.kind_is::<ImageKind>("art"));
    let node = rig.sem("art");
    assert_eq!((node.role, node.name.as_str()), (Role::Image, "Album art"));
    let props = rig.props::<ImageProps>("art");
    let p = ImagePipeline::global();
    let size = props.natural_size(Some(300.), p);
    assert_eq!((size.width, size.height), (300., 150.), "the hint's aspect before any byte is read");
    rig.emit("art", ImageLoaded { width: 1200, height: 600, animated: false });
    assert_eq!(rig.state().loaded.len(), 1);
    let unknown = ImageProps { size_hint: None, source: None, ..props };
    let size = unknown.natural_size(Some(320.), p);
    assert_eq!((size.width, size.height), (320., 240.), "4:3 when nothing is known");
}

#[test]
fn an_equal_image_renders_no_update() {
    let mut rig = Rig::new((), |_, _| image("a.png").id("a").rounded_full().size(px(40.)).into_any_element());
    rig.fresh();
    rig.rerender();
    assert_eq!(rig.all_updates(), 0);
}

#[test]
fn the_clock_extrapolates_while_playing_and_clamps_to_the_length() {
    let at = Instant::now();
    let clock = PlaybackClock::playing(Duration::from_secs(10)).reported_at(at);
    assert_eq!(clock.position_at(at + Duration::from_secs(2), None), Duration::from_secs(12));
    assert_eq!(clock.position_at(at + Duration::from_secs(60), Some(Duration::from_secs(30))), Duration::from_secs(30));
    let paused = PlaybackClock::paused(Duration::from_secs(10)).reported_at(at);
    assert_eq!(paused.position_at(at + Duration::from_secs(5), None), Duration::from_secs(10));
    let fast = clock.rate(2.);
    assert_eq!(fast.position_at(at + Duration::from_secs(1), None), Duration::from_secs(12));
    assert_eq!(format_clock(Duration::from_secs(65)), "1:05");
    assert_eq!(format_clock(Duration::from_secs(3725)), "1:02:05");
    assert_eq!(parse_clock("1:05", None), Some(Duration::from_secs(65)));
    assert_eq!(parse_clock("90", None), Some(Duration::from_secs(90)));
    assert_eq!(parse_clock("50%", Some(Duration::from_secs(200))), Some(Duration::from_secs(100)));
    assert_eq!(parse_clock("soon", None), None);
}

fn seek_props(position: u64) -> SeekBarProps {
    SeekBarProps {
        clock: PlaybackClock::playing(Duration::from_secs(position)),
        duration: Some(Duration::from_secs(200)),
        buffered: Some(Duration::from_secs(120)),
        show_times: true,
        times_beside: false,
        line: false,
        step: Duration::from_secs(5),
        label: "Playback position".into(),
        enabled: true,
        tooltip: None,
        style: Default::default(),
    }
}

#[test]
fn a_drag_shows_the_drag_then_commits_once_and_holds_until_the_clock_catches_up() {
    let props = seek_props(10);
    let mut m = SeekModel::default();
    let now = props.clock.at;
    assert_eq!(m.press(&props, 0.5), Some(Seeked { position: Duration::from_secs(100), phase: SeekPhase::Scrubbing }));
    assert_eq!(m.moved(&props, 0.5), None, "no move, no event");
    assert_eq!(m.moved(&props, 0.25).map(|s| s.position), Some(Duration::from_secs(50)));
    assert_eq!(m.shown(&props, now + Duration::from_secs(3)), Duration::from_secs(50), "the drag wins over the clock");
    let commit = m.release(&props, now).unwrap();
    assert_eq!(commit, Seeked { position: Duration::from_secs(50), phase: SeekPhase::Committed });
    assert_eq!(m.release(&props, now), None, "one commit per drag");
    // The app has not applied the seek yet: its clock still says 10 s.
    assert_eq!(m.shown(&props, now + Duration::from_millis(200)), Duration::from_secs(50), "no snap back");
    // It caught up.
    let caught = seek_props(50);
    let shown = m.shown(&caught, caught.clock.at);
    assert_eq!(shown.as_secs(), 50);
    // A stale clock after the settle time wins again.
    let mut m2 = SeekModel::default();
    m2.press(&props, 0.9);
    m2.release(&props, now);
    assert_eq!(m2.shown(&props, now + Duration::from_secs(2)).as_secs(), 12);
    assert!(props.buffered_fraction(0.1) > 0.59 && props.buffered_fraction(0.9) == 0.9, "buffered never trails played");
}

#[test]
fn arrow_keys_step_and_an_unknown_length_takes_no_seek() {
    let props = seek_props(10);
    let now = props.clock.at;
    let mut m = SeekModel::default();
    assert_eq!(m.step(&props, 1, now).map(|s| s.position), Some(Duration::from_secs(15)));
    assert_eq!(m.step(&props, -4, now).map(|s| s.position), Some(Duration::from_secs(0)), "from the 15 shown, clamped");
    let live = SeekBarProps { duration: None, ..seek_props(10) };
    assert_eq!(m.press(&live, 0.5), None);
    assert_eq!(m.step(&live, 1, now), None);
    let (elapsed, remaining) = SeekModel::default().times(&props, now);
    assert_eq!((elapsed.as_str(), remaining.as_str()), ("0:10", "-3:10"));
}

#[test]
fn times_beside_show_the_length_and_a_placeholder_without_one() {
    let props = SeekBarProps { times_beside: true, ..seek_props(10) };
    let now = props.clock.at;
    let (elapsed, end) = SeekModel::default().times(&props, now);
    assert_eq!((elapsed.as_str(), end.as_str()), ("0:10", "3:20"));
    let live = SeekBarProps { duration: None, ..props };
    let (_, end) = SeekModel::default().times(&live, now);
    assert_eq!(end, "--:--");
}

#[derive(Default)]
struct Player {
    now: Option<NowPlaying>,
    commands: Vec<TransportCommand>,
    seeks: Vec<Seeked>,
}

fn player_view(s: &Player, cx: &mut Context<Host<Player>>) -> AnyElement {
    let now = s.now.clone().unwrap_or_else(|| {
        NowPlaying::new("Song")
            .subtitle("Artist")
            .clock(PlaybackClock::paused(Duration::from_secs(30)))
            .duration(Duration::from_secs(180))
            .buffered(Duration::from_secs(90))
            .volume(0.5, false)
    });
    v_flex()
        .child(player_bar("player", now).on_command(cx.listener(|h, c: &TransportCommand, _, cx| {
            h.s.commands.push(*c);
            cx.notify();
        })))
        .child(
            seek_bar("loose", PlaybackClock::paused(Duration::from_secs(5)))
                .duration(Some(Duration::from_secs(60)))
                .on_seek(cx.listener(|h, e: &Seeked, _, cx| {
                    h.s.seeks.push(*e);
                    cx.notify();
                })),
        )
        .into_any_element()
}

#[test]
fn player_bar_is_built_from_native_controls_and_sends_commands() {
    let mut rig = Rig::new(Player::default(), player_view);
    assert!(rig.kind_is::<SeekBarKind>("player.seek"));
    assert!(rig.kind_is::<ImageKind>("player.art"));
    assert_eq!(rig.sem("player.seek").value.as_deref(), Some("0:30 of 3:00"));
    assert_eq!(rig.sem("player.seek").role, Role::Slider);
    rig.perform("player.play", SemanticAction::Click).unwrap();
    rig.perform("player.next", SemanticAction::Click).unwrap();
    rig.set_value("player.seek", "1:00").unwrap();
    rig.set_value("player.volume", "0.25").unwrap();
    rig.emit("player.seek", Seeked { position: Duration::from_secs(70), phase: SeekPhase::Scrubbing });
    assert_eq!(
        rig.state().commands,
        [
            TransportCommand::TogglePlay,
            TransportCommand::Next,
            TransportCommand::Seek(Duration::from_secs(60)),
            TransportCommand::SetVolume(0.25),
            TransportCommand::Scrub(Duration::from_secs(70)),
        ]
    );
    let _ = SliderChanged { value: 0. };
}

#[test]
fn player_bar_disables_what_the_player_cannot_do() {
    let mut rig = Rig::new(Player::default(), player_view);
    rig.mutate(|s| {
        let mut now = NowPlaying::new("Live").clock(PlaybackClock::playing(Duration::ZERO));
        now.can_next = false;
        now.can_previous = false;
        s.now = Some(now);
    });
    assert!(!rig.sem("player.next").enabled);
    assert!(!rig.sem("player.previous").enabled);
    assert!(refused(rig.perform("player.next", SemanticAction::Click)));
    assert!(!rig.sem("player.seek").enabled, "a live stream takes no seeks");
    assert!(refused(rig.set_value("player.seek", "0:10")));
    assert_eq!(rig.sem("player.play").name, "Pause", "an icon-only button is named by its tooltip");
    assert!(rig.state().commands.is_empty());
}

#[test]
fn seek_bar_set_value_commits_and_feeding_the_clock_back_sets_nothing_new() {
    let mut rig = Rig::new(Player::default(), player_view);
    rig.set_value("loose", "50%").unwrap();
    assert_eq!(rig.state().seeks, [Seeked { position: Duration::from_secs(30), phase: SeekPhase::Committed }]);
    assert!(refused(rig.set_value("loose", "later")));
    rig.fresh();
    rig.rerender();
    assert_eq!(rig.all_updates(), 0, "the same clock re-renders nothing");
}

fn song() -> Lyrics {
    let s = Duration::from_secs;
    Lyrics::synced([
        LyricLine::new(s(5), "First line here").end(s(8)),
        LyricLine::new(s(9), "Second line").end(s(12)),
        LyricLine::new(s(20), "After a break"),
        LyricLine::new(s(24), "Last"),
    ])
}

#[test]
fn lyric_rows_get_interludes_and_word_timing() {
    let rows = lyric_rows(&song());
    let kinds: Vec<LyricRowKind> = rows.iter().map(|r| r.kind).collect();
    use LyricRowKind::*;
    assert_eq!(kinds, [Interlude, Line, Line, Interlude, Line, Line], "an intro over 3 s and an 8 s gap");
    let first = &rows[1];
    assert_eq!(first.words.len(), 3);
    assert!((first.words[0].start - 5.).abs() < 1e-9);
    // "First" is 5 of 13 characters over 3 s.
    assert!((first.words[1].start - (5. + 3. * 5. / 13.)).abs() < 1e-6);
    assert_eq!(word_fill(&first.words[0], 4.), 0.);
    assert_eq!(word_fill(&first.words[0], 100.), 1.);
    assert!((interlude_fill(&rows[0], 2.5) - 0.5).abs() < 1e-6);
    let plain = lyric_rows(&Lyrics::plain("one\n\ntwo"));
    assert_eq!(plain.len(), 2, "blank lines are dropped");
}

#[test]
fn the_lit_line_carries_across_short_gaps_and_the_anchor_survives_long_ones() {
    let rows = lyric_rows(&song());
    let at = |t: f64, prev: usize| lit_at(&rows, t, prev);
    assert_eq!(at(6., 0).anchor, 1);
    assert!(at(6., 0).contains(1));
    assert!(at(8.5, 1).contains(1), "ended at 8 s, the next starts within 3 s: still lit");
    assert_eq!(at(10., 1).anchor, 2);
    let gap = at(15., 2);
    assert!(gap.contains(3), "the interlude is lit in the gap");
    assert_eq!(gap.anchor, 3);
    assert_eq!(at(30., 4).anchor, 5);
}

#[test]
fn follow_stops_on_a_user_scroll_and_resumes_by_button_or_timeout() {
    let mut f = Follow::default();
    let t0 = Instant::now();
    assert!(f.following());
    assert!(f.user_scrolled(t0));
    assert!(!f.user_scrolled(t0), "already off");
    assert!(!f.tick(t0 + Duration::from_secs(60), None), "no timeout: waits for the button");
    assert!(!f.tick(t0 + Duration::from_secs(1), Some(Duration::from_secs(5))));
    assert!(f.tick(t0 + Duration::from_secs(6), Some(Duration::from_secs(5))));
    assert!(f.following());
    f.user_scrolled(t0);
    assert!(f.resync());
    assert!(!f.resync());
}

#[test]
fn lyrics_depth_and_curves() {
    assert_eq!(depth_opacity(0, 6.), 1.);
    assert!((depth_opacity(1, 6.) - 0.85).abs() < 1e-6);
    assert_eq!(depth_opacity(40, 6.), 0.25);
    assert!((bezier(SCROLL_CURVE, 0.) - 0.).abs() < 1e-3);
    assert!((bezier(SCROLL_CURVE, 1.) - 1.).abs() < 1e-3);
    assert!((0..=20).map(|i| bezier(SCROLL_CURVE, i as f32 / 20.)).any(|v| v > 1.), "the scroll overshoots");
    assert_eq!(anchor_y(500.), 210.);
    assert_eq!(follow_offset(400., 500.), 190.);
}

#[derive(Default)]
struct Sing {
    clicks: Vec<LyricClicked>,
    follow: Vec<bool>,
}

#[test]
fn lyrics_view_reports_the_current_line_and_clicks_seek() {
    let mut rig = Rig::new(Sing::default(), |_, cx| {
        lyrics_view("lyrics", song(), PlaybackClock::paused(Duration::from_secs(10)))
            .on_line_click(cx.listener(|h, e: &LyricClicked, _, cx| {
                h.s.clicks.push(*e);
                cx.notify();
            }))
            .on_follow_change(cx.listener(|h, e: &LyricsFollowChanged, _, cx| {
                h.s.follow.push(e.following);
                cx.notify();
            }))
            .size_full()
            .into_any_element()
    });
    assert!(rig.kind_is::<LyricsKind>("lyrics"));
    let node = rig.sem("lyrics");
    assert_eq!((node.role, node.value.as_deref()), (Role::List, Some("Second line")));
    rig.emit("lyrics", LyricClicked { line: Some(2), start: Duration::from_secs(20) });
    rig.emit("lyrics", LyricsFollowChanged { following: false });
    assert_eq!(rig.state().clicks[0].start, Duration::from_secs(20));
    assert_eq!(rig.state().follow, [false]);
    let mut model = LyricsModel::default();
    let props = rig.props::<LyricsProps>("lyrics");
    assert!(model.set_lyrics(&props.lyrics));
    assert!(!model.set_lyrics(&props.lyrics), "equal lyrics keep the rows and follow state");
    assert!(model.advance(&props, Instant::now()));
    assert_eq!(model.lit.anchor, 2);
}

fn cards(n: usize, w: f32, gap: f32, inset: f32) -> Vec<Rect> {
    (0..n).map(|i| Rect::new(inset + i as f32 * (w + gap), 0., w, w)).collect()
}

#[test]
fn shelf_pages_by_the_viewport_less_a_card_and_lands_on_card_edges() {
    let c = cards(10, 160., 20., 0.);
    let max = 10. * 180. - 20. - 600.;
    let page = scroll_target(ShelfScroll::Page(1), 0., 600., max, &c, 0.);
    assert_eq!(page, 360., "440 forward, back to the card edge at 360");
    assert_eq!(scroll_target(ShelfScroll::Page(-1), page, 600., max, &c, 0.), 0.);
    assert_eq!(scroll_target(ShelfScroll::Item(3), 0., 600., max, &c, 0.), 540.);
    assert_eq!(scroll_target(ShelfScroll::Item(9), 0., 600., max, &c, 0.), max, "clamped to the end");
    assert_eq!(snap_offset(250., &c, 0., max, 0), 180., "nearest edge");
    assert_eq!(snap_offset(max - 5., &c, 0., max, 0), max, "the end is a stop");
}

#[derive(Default)]
struct Shelves {
    handle: ShelfHandle,
}

#[test]
fn a_shelf_request_reaches_the_backend_once_and_a_rerender_sends_nothing() {
    let mut rig = Rig::new(Shelves::default(), |s, _| {
        shelf("recent", &s.handle)
            .label("Recently played")
            .gap_4()
            .children((0..6).map(|i| div().id(format!("card{i}")).size(px(120.))))
            .into_any_element()
    });
    assert_eq!(rig.sem("recent").role, Role::ScrollArea);
    assert_eq!(rig.sem("recent").name, "Recently played");
    rig.fresh();
    rig.rerender();
    assert_eq!(rig.updates("recent"), 0);
    rig.state().handle.page(1);
    rig.rerender();
    assert_eq!(rig.updates("recent"), 1, "the new request is a prop change");
    let (seq, scroll) = rig.state().handle.pending().unwrap();
    assert_eq!(scroll, ShelfScroll::Page(1));
    rig.state().handle.applied(seq);
    assert!(rig.state().handle.pending().is_none());
    rig.state().handle.report(100., 300., 480.);
    assert!(rig.state().handle.can_page_back() && rig.state().handle.can_page_forward());
}

#[test]
fn runs_tighten_shared_corners_and_only_the_last_bubble_has_a_tail() {
    assert_eq!(RunPosition::of(false, false), RunPosition::Single);
    assert_eq!(RunPosition::of(true, true), RunPosition::Middle);
    assert_eq!(bubble_corners(BubbleSide::Sent, RunPosition::Middle), [18., 5., 5., 18.]);
    assert_eq!(bubble_corners(BubbleSide::Received, RunPosition::First), [18., 18., 18., 5.]);
    assert_eq!(bubble_corners(BubbleSide::Received, RunPosition::Last), [5., 18., 18., 18.]);
    assert!(is_emoji_only("\u{1f600}\u{1f600}", 3));
    assert!(is_emoji_only("\u{1f44d}\u{1f3fd}", 3), "a skin tone is part of its emoji");
    assert!(is_emoji_only("\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}", 1), "a family is one emoji");
    assert!(is_emoji_only("\u{1f1ea}\u{1f1f8}\u{1f1eb}\u{1f1f7}", 2), "two flags");
    assert!(!is_emoji_only("\u{1f600}\u{1f600}\u{1f600}\u{1f600}", 3));
    assert!(!is_emoji_only("ok \u{1f600}", 3));
    assert_eq!(initials("Ada Lovelace"), "AL");
    assert_eq!(initials("grace brewster hopper"), "GH");
    assert_eq!(initials("Plato"), "P");
    assert_eq!(initials("+34 600 000 000"), "#");
    assert_eq!(initials(""), "#");
    assert_eq!(group_layout(3, 40.).len(), 3);
    let four = group_layout(6, 40.);
    assert_eq!(four.len(), 4, "a group shows its first four");
    assert!(four.iter().all(|r| r.origin.x >= 0. && r.origin.x + r.size.width <= 40.));
}

#[test]
fn chat_rows_show_the_sender_once_per_run() {
    let rig = Rig::new((), |_, _| {
        let ada = Person::new("Ada Lovelace");
        v_flex()
            .child(chat_row("m1", ChatMessage::received("Hi").sender(ada.clone()).position(RunPosition::First)))
            .child(chat_row(
                "m2",
                ChatMessage::received("Are you there?").sender(ada).position(RunPosition::Last).time("9:41"),
            ))
            .child(chat_row(
                "m3",
                ChatMessage::sent("\u{1f44b}")
                    .reactions([Reaction { glyph: "\u{2764}".into(), count: 2, mine: true }])
                    .receipt("Delivered"),
            ))
            .child(
                avatar_group([Person::new("A B"), Person::new("C"), Person::new("D"), Person::new("E")], 40.)
                    .id("group"),
            )
            .into_any_element()
    });
    assert!(rig.kind_is::<ChatBubbleKind>("m1"));
    assert!(
        rig.cx.semantic_tree().nodes().all(|(_, n)| n.test_id.as_deref() != Some("m1.avatar")),
        "not the run's end"
    );
    assert!(rig.kind_is::<AvatarKind>("m2.avatar"));
    assert_eq!(rig.sem("m2.avatar").name, "Ada Lovelace");
    assert_eq!(rig.props::<ChatBubbleProps>("m2").tooltip.as_deref(), Some("9:41"));
    let wave = rig.props::<ChatBubbleProps>("m3");
    assert!(wave.emoji_only && !wave.has_tail());
    assert_eq!(rig.sem("m3").value.as_deref(), Some("Sent, \u{2764} 2"));
    assert_eq!(rig.sem("group").name, "A B, C and 2 more");
    let _ = ClickEvent;
}

#[derive(Default)]
struct Chat {
    draft: Draft,
    sent: Vec<Outgoing>,
    attach: u32,
}

fn chat_view(s: &Chat, cx: &mut Context<Host<Chat>>) -> AnyElement {
    composer("composer", &s.draft)
        .on_event(cx.listener(|h, e: &ComposerEvent, _, cx| {
            if *e == ComposerEvent::Attach {
                h.s.attach += 1;
            }
            if let Some(out) = h.s.draft.apply(e) {
                h.s.sent.push(out);
            }
            cx.notify();
        }))
        .into_any_element()
}

#[test]
fn composer_field_grows_submits_on_return_and_takes_media() {
    let mut rig = Rig::new(Chat::default(), chat_view);
    let field = rig.props::<TextAreaProps>("composer.field");
    assert!(field.submit_on_return && field.accepts_media);
    assert_eq!((field.min_lines, field.max_lines), (1, Some(COMPOSER_LINES)));
    assert_eq!(field.placeholder.as_deref(), Some("Message"));
    let has = |rig: &Rig<Chat>, id: &str| rig.cx.semantic_tree().nodes().any(|(_, n)| n.test_id.as_deref() == Some(id));
    assert!(!has(&rig, "composer.send"), "nothing to send: no send button, as in Messages");
    rig.perform("composer.field", SemanticAction::Type("hello ".into())).unwrap();
    assert_eq!(rig.state().draft.text, "hello ");
    assert!(rig.sem("composer.send").enabled);
    for id in ["composer.attach", "composer.emoji", "composer.send"] {
        let b = rig.sem(id).bounds.expect("laid out");
        assert_eq!((b.size.width, b.size.height), (COMPOSER_ROW, COMPOSER_ROW), "{id} is one line of the field square");
    }
    assert!(field.style.corner_radii.top_left.is_some(), "the field is a capsule");
    rig.emit("composer.field", TextSubmitted { text: "hello ".into() });
    assert_eq!(rig.state().sent.len(), 1);
    assert_eq!(rig.state().sent[0].text.as_deref(), Some("hello"));
    assert_eq!(rig.state().draft.text, "", "sending clears the draft");
    assert_eq!(rig.props::<TextAreaProps>("composer.field").text, "");
    rig.emit("composer.field", TextSubmitted { text: "".into() });
    assert_eq!(rig.state().sent.len(), 1, "an empty draft sends nothing");
}

#[test]
fn composer_stages_pasted_images_and_sends_them_with_the_text() {
    let mut rig = Rig::new(Chat::default(), chat_view);
    let pasted = PastedImage { bytes: Arc::from(png(8, 8)), mime: "image/png".into() };
    rig.emit("composer.field", ImagesPasted { images: vec![pasted.clone(), pasted] });
    assert_eq!(rig.state().draft.attachments.len(), 2);
    let first = rig.state().draft.attachments[0].id.clone();
    assert!(rig.kind_is::<ImageKind>(&format!("composer.thumb.{first}")));
    assert!(rig.sem("composer.send").enabled, "attachments alone can send");
    rig.perform(&format!("composer.remove.{first}"), SemanticAction::Click).unwrap();
    assert_eq!(rig.state().draft.attachments.len(), 1);
    rig.perform("composer.attach", SemanticAction::Click).unwrap();
    assert_eq!(rig.state().attach, 1);
    rig.perform("composer.emoji", SemanticAction::Click).unwrap();
    assert_eq!(rig.props::<TextAreaProps>("composer.field").emoji_picker, 1, "the request reaches the field");
    rig.perform("composer.send", SemanticAction::Click).unwrap();
    let sent = &rig.state().sent[0];
    assert_eq!((sent.text.clone(), sent.attachments.len()), (None, 1));
    assert!(rig.state().draft.attachments.is_empty());
}

#[test]
fn dropped_files_preview_images_by_extension() {
    let mut d = Draft::default();
    d.apply(&ComposerEvent::Files(vec!["/tmp/photo.JPG".into(), "/tmp/notes.pdf".into()]));
    assert!(d.attachments[0].preview.is_some());
    assert!(d.attachments[1].preview.is_none());
    assert_eq!(d.attachments[1].name, "notes.pdf");
    assert_ne!(d.attachments[0].id, d.attachments[1].id);
}

#[test]
fn svg_data_urls_render_at_the_box_size_and_take_a_tint() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="20"><rect width="10" height="20" fill="#ff0000"/></svg>"##;
    let source = ImageSource::from(format!("data:image/svg+xml;utf8,{}", svg.replace('#', "%23")).as_str());
    let ImageSource::Bytes { bytes, .. } = &source else { panic!("a data URL is bytes") };
    assert_eq!(&bytes[..], svg.as_bytes());
    let p = ImagePipeline::new(PipelineConfig { disk_dir: None, workers: 1, ..PipelineConfig::default() });
    assert_eq!(p.intrinsic_size(&source), Some((10, 20)), "the header of an SVG is its size");

    let request = DecodeRequest::at(source.clone(), 64., 64., 1., ImageFit::Contain);
    let d = decode(bytes, &request, 1 << 30).unwrap();
    assert_eq!((d.width, d.height), (32, 64), "a vector scales up to its box");
    assert_eq!(&d.frames[0].rgba[..4], &[255, 0, 0, 255]);

    let tinted = decode(bytes, &request.clone().tinted(Some(hsla(0.6, 1., 0.5, 1.))), 1 << 30).unwrap();
    let px = &tinted.frames[0].rgba[..4];
    assert!(px[2] > 200 && px[0] < 40 && px[3] == 255, "every pixel takes the tint: {px:?}");
    assert_ne!(request, request.clone().tinted(Some(hsla(0.6, 1., 0.5, 1.))), "a tint is its own cache entry");

    use base64::Engine;
    let b64 = format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(svg));
    let ImageSource::Bytes { bytes, .. } = ImageSource::from(b64.as_str()) else { panic!() };
    assert_eq!(&bytes[..], svg.as_bytes());
    assert_eq!(load(&p, request).unwrap().width, 32, "the pipeline decodes it off the main thread");
}

#[derive(Default)]
struct Clips {
    playing: bool,
    ready: Vec<VideoReady>,
    ended: u32,
    failed: Vec<String>,
}

#[test]
fn video_carries_play_state_as_props_and_reports_its_events() {
    let mut rig = Rig::new(Clips::default(), |s, cx| {
        video("/tmp/clip.mp4")
            .id("clip")
            .alt("Clip")
            .playing(s.playing)
            .muted(true)
            .size(px(200.))
            .on_ready(cx.listener(|h, e: &VideoReady, _, cx| {
                h.s.ready.push(*e);
                cx.notify();
            }))
            .on_end(cx.listener(|h, _: &VideoEnded, _, cx| {
                h.s.ended += 1;
                cx.notify();
            }))
            .on_error(cx.listener(|h, e: &VideoFailed, _, cx| {
                h.s.failed.push(e.error.clone());
                cx.notify();
            }))
            .into_any_element()
    });
    assert!(rig.kind_is::<VideoKind>("clip"));
    let node = rig.sem("clip");
    assert_eq!((node.role, node.name.as_str()), (Role::Image, "Clip"));
    let props = rig.props::<VideoProps>("clip");
    assert_eq!(props.source.as_deref(), Some("/tmp/clip.mp4"));
    assert!(!props.playing && props.muted && !props.looping);
    assert_eq!(props.fit, ImageFit::Contain, "letterboxed unless asked to crop");
    rig.mutate(|s| s.playing = true);
    assert!(rig.props::<VideoProps>("clip").playing);
    rig.emit("clip", VideoReady { width: 640, height: 360, duration: Some(3.5) });
    rig.emit("clip", VideoEnded);
    rig.emit("clip", VideoFailed { error: "gone".into() });
    rig.cx.run_until_idle();
    let s = rig.state();
    assert_eq!((s.ready.len(), s.ended, s.failed.as_slice()), (1, 1, &["gone".to_string()][..]));
}
