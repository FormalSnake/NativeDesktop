//! Every `nd::system` service behind a button, and every OS event the app receives in a log:
//! activation, `nd-system-demo://` URLs, `.ndsys` documents, files dropped on the window,
//! notification and tray clicks, media keys, and the audio player's states with its spectrum
//! drawn as bars.

use std::time::Duration;

use nd::prelude::*;
use nd::system::{
    AudioEvent, ClipboardImage, MediaCommand, MessageLevel, MessageOptions, Notification, NowPlaying, OpenOptions,
    PlayOptions, PlaybackState, Player, SaveOptions, System, SystemEvent, Tray, TrayIcon, TrayId, TrayMenuItem,
};

const SECRET_SERVICE: &str = "dev.formalsnake.system-demo";
const TRAY: &str = "system-demo";

pub struct SystemDemo {
    sys: System,
    log: Vec<String>,
    tray: bool,
    player: Option<Player>,
    playing: PlaybackState,
    spectrum: Vec<f32>,
}

pub fn app() -> Application {
    Application::new(nd::manifest!())
}

pub fn open(cx: &mut App, sys: System) -> WindowHandle<SystemDemo> {
    let events = sys.clone();
    let handle = cx.open_window(WindowOptions::main("main").title("System").size(size(px(760.), px(640.))), |_, cx| {
        cx.new(|_| SystemDemo {
            sys,
            log: Vec::new(),
            tray: false,
            player: None,
            playing: PlaybackState::Stopped,
            spectrum: Vec::new(),
        })
    });
    let view = handle.root().downgrade();
    // docs:begin subscribe
    nd::system::on_event(&events, move |event, app| {
        view.update(app, |this, cx| {
            this.event(event);
            cx.notify();
        });
    });
    // docs:end subscribe
    handle
}

/// A one-second 440 Hz tone as 16-bit mono WAV.
fn tone_wav() -> Vec<u8> {
    let rate = 44_100u32;
    let samples: Vec<i16> =
        (0..rate).map(|i| ((i as f32 * 440. * std::f32::consts::TAU / rate as f32).sin() * 12_000.) as i16).collect();
    let data_len = samples.len() as u32 * 2;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        wav.extend_from_slice(&s.to_le_bytes());
    }
    wav
}

impl SystemDemo {
    pub fn log(&self) -> &[String] {
        &self.log
    }

    fn note(&mut self, line: impl Into<String>) {
        self.log.push(line.into());
        let excess = self.log.len().saturating_sub(12);
        self.log.drain(..excess);
    }

    // docs:begin events
    fn event(&mut self, event: &SystemEvent) {
        match event {
            SystemEvent::Audio { event: AudioEvent::Spectrum(bins), .. } => self.spectrum = bins.clone(),
            SystemEvent::Audio { event: AudioEvent::State { state, .. }, .. } => {
                self.note(format!("audio: {state:?}"));
                self.playing = match state {
                    nd::system::AudioState::Playing => PlaybackState::Playing,
                    nd::system::AudioState::Paused => PlaybackState::Paused,
                    _ => {
                        self.spectrum.clear();
                        PlaybackState::Stopped
                    }
                };
            }
            SystemEvent::Media(MediaCommand::Toggle | MediaCommand::Play | MediaCommand::Pause) => {
                self.note(format!("{event:?}"));
                if let Some(player) = &self.player {
                    let wants_play = self.playing != PlaybackState::Playing;
                    nd::system::spawn(if wants_play { player.resume() } else { player.pause() }, |_, _| {});
                }
            }
            SystemEvent::Tray { item: Some(item), .. } if item == "quit-tray" => {
                self.note("tray: hide");
                self.tray = false;
                nd::system::spawn(self.sys.tray().remove(TrayId(TRAY.into())), |_, _| {});
            }
            other => self.note(format!("{other:?}")),
        }
    }
    // docs:end events

    // docs:begin spawn
    /// Runs `call` and logs its outcome under `what`.
    fn run<T: std::fmt::Debug + Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        what: &'static str,
        call: impl std::future::Future<Output = nd::system::Result<T>> + Send + 'static,
    ) {
        let view = cx.weak_entity();
        nd::system::spawn(call, move |result, app| {
            view.update(app, |this, cx| {
                match result {
                    Ok(value) => this.note(format!("{what}: {value:?}")),
                    Err(e) => this.note(format!("{what} failed: {e}")),
                }
                cx.notify();
            });
        });
    }
    // docs:end spawn

    fn row(
        &self,
        cx: &mut Context<Self>,
        title: &str,
        buttons: Vec<(&'static str, &'static str, Action)>,
    ) -> impl IntoElement {
        let secondary = cx.theme().text.secondary;
        let mut row = h_flex().gap_2().items_center().child(text(title.to_owned()).text_color(secondary).w(px(110.)));
        for (id, label, action) in buttons {
            row = row.child(
                button(id, label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.act(action, window, cx))),
            );
        }
        row
    }

    fn act(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        let sys = self.sys.clone();
        match action {
            // docs:begin dialogs
            Action::Open => {
                self.run(cx, "open", sys.dialogs().open(OpenOptions::file().multiple().title("Pick files")))
            }
            Action::Folder => self.run(cx, "folder", sys.dialogs().open(OpenOptions::folder())),
            Action::Save => self.run(cx, "save", sys.dialogs().save(SaveOptions::new().file_name("notes.ndsys"))),
            // docs:end dialogs
            Action::Alert => {
                let alert = MessageOptions::new("Empty the cache?")
                    .detail("The app downloads it again when it needs it.")
                    .level(MessageLevel::Warning)
                    .button("Empty")
                    .button("Cancel");
                self.run(cx, "alert", sys.dialogs().message(alert));
            }
            Action::Sheet => {
                let view = cx.weak_entity();
                let alert = Alert::new("A sheet on this window")
                    .message("Window dialogs come from nd::system::window.")
                    .button("OK");
                nd::system::spawn(nd::system::window::alert(cx, window.id(), alert), move |answer, app| {
                    view.update(app, |this, cx| {
                        this.note(format!("sheet: {answer:?}"));
                        cx.notify();
                    });
                });
            }
            Action::CopyText => self.run(cx, "copy", sys.clipboard().write_text("Copied from the System demo")),
            Action::PasteText => self.run(cx, "paste", sys.clipboard().read_text()),
            // docs:begin clipboard
            Action::CopyImage => {
                let rgba = (0..64 * 64).flat_map(|i| [(i % 64 * 4) as u8, (i / 64 * 4) as u8, 200, 255]).collect();
                self.run(cx, "copy image", sys.clipboard().write_image(ClipboardImage { width: 64, height: 64, rgba }));
            }
            Action::PasteImage => {
                let image = sys.clipboard().read_image();
                self.run(cx, "paste image", async move { image.await.map(|i| i.map(|i| (i.width, i.height))) });
            }
            // docs:end clipboard
            Action::PasteFiles => self.run(cx, "paste files", sys.clipboard().read_files()),
            // docs:begin notify
            Action::Notify => {
                let n = Notification::new("System demo")
                    .subtitle("A notification")
                    .body("Click it, or one of its buttons.")
                    .action("reply", "Reply")
                    .data(serde_json::json!({ "from": "the demo" }));
                self.run(cx, "notify", sys.notifications().show(n));
                // docs:end notify
            }
            // docs:begin credentials
            Action::Remember => self.run(cx, "remember", sys.credentials().set(SECRET_SERVICE, "demo", "hunter2")),
            Action::Recall => self.run(cx, "recall", sys.credentials().get(SECRET_SERVICE, "demo")),
            Action::Forget => self.run(cx, "forget", sys.credentials().delete(SECRET_SERVICE, "demo")),
            // docs:end credentials
            Action::Recent => {
                let file = nd::system::app_dirs().ensure_data().map(|dir| dir.join("example.ndsys"));
                let written = file.and_then(|f| nd::files::write(&f, b"a demo document").map(|()| f));
                let Ok(file) = written else {
                    return self.note(format!("recent failed: {written:?}"));
                };
                self.run(cx, "recent", sys.recent_documents().add(file));
            }
            Action::Reveal => {
                let result = nd::system::app_dirs().ensure_data().map(|data| sys.shell().reveal_path(data));
                self.note(format!("reveal data: {result:?}"));
            }
            Action::Website => {
                let result = sys.shell().open_url("https://github.com/FormalSnake");
                self.note(format!("open url: {result:?}"));
            }
            // docs:begin tray
            Action::Tray => {
                self.tray = !self.tray;
                if self.tray {
                    let tray = Tray::new(TRAY)
                        .icon(TrayIcon::Named(if cfg!(target_os = "macos") {
                            "waveform".into()
                        } else {
                            "audio-x-generic".into()
                        }))
                        .tooltip("System demo")
                        .item(TrayMenuItem::item("hello", "Say hello"))
                        .item(TrayMenuItem::check("checked", "A checked item", true))
                        .item(TrayMenuItem::Separator)
                        .item(TrayMenuItem::item("quit-tray", "Hide this icon"));
                    self.run(cx, "tray", sys.tray().set(tray));
                } else {
                    self.run(cx, "tray removed", sys.tray().remove(TrayId(TRAY.into())));
                }
            }
            // docs:end tray
            // docs:begin audio
            Action::Play => {
                let path = nd::system::app_dirs().ensure_cache().map(|dir| dir.join("tone.wav"));
                let written = path.and_then(|p| nd::files::write(&p, tone_wav()).map(|()| p));
                let Ok(path) = written else {
                    return self.note(format!("play failed: {written:?}"));
                };
                let (player, started) = sys.audio().play(PlayOptions::path(path).volume(0.3).spectrum());
                self.player = Some(player);
                let media = sys.media().set(NowPlaying {
                    title: "A 440 Hz tone".into(),
                    artist: Some("System demo".into()),
                    duration: Some(Duration::from_secs(1)),
                    state: PlaybackState::Playing,
                    ..NowPlaying::default()
                });
                let view = cx.weak_entity();
                nd::system::spawn(
                    async move {
                        started.await?;
                        let _ = media.await;
                        Ok::<_, nd::system::Error>(())
                    },
                    move |result, app| {
                        view.update(app, |this, cx| {
                            if let Err(e) = result {
                                this.player = None;
                                this.note(format!("play failed: {e}"));
                            }
                            cx.notify();
                        });
                    },
                );
            }
            // docs:end audio
            Action::Stop => {
                if let Some(player) = self.player.take() {
                    self.run(cx, "stop", player.stop());
                }
                self.run(cx, "now playing cleared", sys.media().clear());
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Action {
    Open,
    Folder,
    Save,
    Alert,
    Sheet,
    CopyText,
    PasteText,
    CopyImage,
    PasteImage,
    PasteFiles,
    Notify,
    Remember,
    Recall,
    Forget,
    Recent,
    Reveal,
    Website,
    Tray,
    Play,
    Stop,
}

impl Render for SystemDemo {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (accent, secondary, margin) = (theme.accent, theme.text.secondary, theme.spacing.content_margin);
        let active = if self.sys.is_active() { "active" } else { "in the background" };
        let bars = h_flex()
            .h(px(48.))
            .items_end()
            .gap_1()
            .children(self.spectrum.iter().map(|b| div().w(px(6.)).h(px(2. + b * 46.)).bg(accent)));
        let tray_label = if self.tray { "Hide tray icon" } else { "Show tray icon" };
        // docs:begin has
        // A target decides the answer, so no window or context is needed to ask.
        let tray_row = nd::platform::has(nd::platform::Feature::Tray)
            .then(|| self.row(cx, "Tray", vec![("tray", tray_label, Action::Tray)]).into_any_element());
        // docs:end has
        scaffold().toolbar(toolbar().title("System")).content(
            v_flex()
                .size_full()
                .p(margin)
                .gap_2()
                .child(
                    text(format!("The app is {active}. Drop files on this window, or open nd-system-demo://hello."))
                        .text_color(secondary),
                )
                .child(self.row(
                    cx,
                    "Dialogs",
                    vec![
                        ("open", "Open…", Action::Open),
                        ("folder", "Folder…", Action::Folder),
                        ("save", "Save…", Action::Save),
                        ("alert", "Alert", Action::Alert),
                        ("sheet", "Sheet", Action::Sheet),
                    ],
                ))
                .child(self.row(
                    cx,
                    "Clipboard",
                    vec![
                        ("copy-text", "Copy text", Action::CopyText),
                        ("paste-text", "Paste text", Action::PasteText),
                        ("copy-image", "Copy image", Action::CopyImage),
                        ("paste-image", "Paste image", Action::PasteImage),
                        ("paste-files", "Paste files", Action::PasteFiles),
                    ],
                ))
                .child(self.row(cx, "Notify", vec![("notify", "Show notification", Action::Notify)]))
                .child(self.row(
                    cx,
                    "Keychain",
                    vec![
                        ("remember", "Remember", Action::Remember),
                        ("recall", "Recall", Action::Recall),
                        ("forget", "Forget", Action::Forget),
                    ],
                ))
                .child(self.row(
                    cx,
                    "Shell",
                    vec![
                        ("recent", "Add to recents", Action::Recent),
                        ("reveal", "Reveal data", Action::Reveal),
                        ("website", "Open website", Action::Website),
                    ],
                ))
                .children(tray_row)
                .child(self.row(cx, "Audio", vec![("play", "Play tone", Action::Play), ("stop", "Stop", Action::Stop)]))
                .child(bars)
                .child(
                    v_flex()
                        .gap_1()
                        .children(self.log.iter().rev().map(|line| text(line.clone()).text_color(secondary))),
                ),
        )
    }
}
