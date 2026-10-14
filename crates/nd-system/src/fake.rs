//! A backend for tier 1 tests. The clipboard, credentials, recent documents, trays, media
//! session and players are in-memory state the test reads back through [`FakeHandle`];
//! dialogs answer from queued answers (cancel when none is queued); every call is recorded;
//! [`FakeHandle::emit`] plays the OS's part.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::backend::{AudioControl, Backend, Emitter, Reply};
use crate::{
    AudioEvent, AudioState, ClipboardImage, Error, Feature, MessageOptions, Notification, NotificationId, NowPlaying,
    OpenOptions, PlayOptions, PlayerId, Result, SaveOptions, SystemEvent, Tray, TrayId,
};

/// One recorded call.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    Open(OpenOptions),
    Save(SaveOptions),
    Message(MessageOptions),
    Notify(NotificationId, Notification),
    OpenUrl(String),
    OpenPath(PathBuf),
    RevealPath(PathBuf),
    AudioControl(PlayerId, AudioControl),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FakePlayer {
    pub options: PlayOptions,
    pub state: AudioState,
    pub position: Duration,
    pub volume: f32,
}

#[derive(Default)]
pub struct FakeState {
    pub calls: Vec<Call>,
    pub open_answers: VecDeque<Option<Vec<PathBuf>>>,
    pub save_answers: VecDeque<Option<PathBuf>>,
    pub message_answers: VecDeque<usize>,
    pub clipboard_text: Option<String>,
    pub clipboard_image: Option<ClipboardImage>,
    pub clipboard_files: Option<Vec<PathBuf>>,
    pub credentials: HashMap<(String, String), String>,
    pub recent: Vec<PathBuf>,
    pub trays: BTreeMap<String, Tray>,
    pub now_playing: Option<NowPlaying>,
    pub players: BTreeMap<u64, FakePlayer>,
    /// Features that answer [`Error::Unsupported`], to test an app's fallbacks.
    pub unsupported: Vec<Feature>,
}

#[derive(Clone, Default)]
struct Shared {
    state: Arc<Mutex<FakeState>>,
    emitter: Arc<Mutex<Option<Emitter>>>,
}

pub struct FakeBackend(Shared);

impl FakeBackend {
    pub fn new() -> Self {
        Self(Shared::default())
    }

    pub fn handle(&self) -> FakeHandle {
        FakeHandle(self.0.clone())
    }

    fn state(&self) -> MutexGuard<'_, FakeState> {
        self.0.state.lock().expect("fake state")
    }

    /// Answers `reply` with `f`'s result, or Unsupported when the test turned `feature` off.
    fn answer<T>(&self, feature: Feature, reply: Reply<T>, f: impl FnOnce(&mut FakeState) -> Result<T>) {
        let mut state = self.state();
        if state.unsupported.contains(&feature) {
            return reply.unsupported(feature);
        }
        reply.send(f(&mut state));
    }

    fn emit(&self, event: SystemEvent) {
        if let Some(e) = &*self.0.emitter.lock().expect("fake emitter") {
            e.emit(event);
        }
    }
}

impl Default for FakeBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// Scripts and inspects a [`FakeBackend`] from the test.
#[derive(Clone)]
pub struct FakeHandle(Shared);

impl FakeHandle {
    pub fn state(&self) -> MutexGuard<'_, FakeState> {
        self.0.state.lock().expect("fake state")
    }

    pub fn calls(&self) -> Vec<Call> {
        self.state().calls.clone()
    }

    pub fn answer_open(&self, paths: Option<Vec<PathBuf>>) {
        self.state().open_answers.push_back(paths);
    }

    pub fn answer_save(&self, path: Option<PathBuf>) {
        self.state().save_answers.push_back(path);
    }

    pub fn answer_message(&self, button: usize) {
        self.state().message_answers.push_back(button);
    }

    pub fn set_unsupported(&self, feature: Feature) {
        self.state().unsupported.push(feature);
    }

    /// Delivers `event` as the OS would.
    pub fn emit(&self, event: SystemEvent) {
        if let Some(e) = &*self.0.emitter.lock().expect("fake emitter") {
            e.emit(event);
        }
    }

    /// Plays a player to its end, emitting the `Ended` transition.
    pub fn finish(&self, player: PlayerId) {
        let event = {
            let mut state = self.state();
            let Some(p) = state.players.get_mut(&player.0) else { return };
            p.state = AudioState::Ended;
            AudioEvent::State { state: AudioState::Ended, position: p.position, duration: None, error: None }
        };
        self.emit(SystemEvent::Audio { player, event });
    }
}

impl Backend for FakeBackend {
    fn attach(&self, events: Emitter) {
        *self.0.emitter.lock().expect("fake emitter") = Some(events);
    }

    fn open_dialog(&self, options: OpenOptions, reply: Reply<Option<Vec<PathBuf>>>) {
        self.answer(Feature::Dialogs, reply, |s| {
            s.calls.push(Call::Open(options));
            Ok(s.open_answers.pop_front().flatten())
        })
    }

    fn save_dialog(&self, options: SaveOptions, reply: Reply<Option<PathBuf>>) {
        self.answer(Feature::Dialogs, reply, |s| {
            s.calls.push(Call::Save(options));
            Ok(s.save_answers.pop_front().flatten())
        })
    }

    fn message(&self, options: MessageOptions, reply: Reply<usize>) {
        self.answer(Feature::Dialogs, reply, |s| {
            let cancel = options.buttons.len().saturating_sub(1);
            s.calls.push(Call::Message(options));
            Ok(s.message_answers.pop_front().unwrap_or(cancel))
        })
    }

    fn clipboard_read_text(&self, reply: Reply<Option<String>>) {
        self.answer(Feature::Clipboard, reply, |s| Ok(s.clipboard_text.clone()))
    }

    fn clipboard_write_text(&self, text: String, reply: Reply<()>) {
        self.answer(Feature::Clipboard, reply, |s| {
            s.clipboard_image = None;
            s.clipboard_files = None;
            s.clipboard_text = Some(text);
            Ok(())
        })
    }

    fn clipboard_read_image(&self, reply: Reply<Option<ClipboardImage>>) {
        self.answer(Feature::ClipboardRich, reply, |s| Ok(s.clipboard_image.clone()))
    }

    fn clipboard_write_image(&self, image: ClipboardImage, reply: Reply<()>) {
        self.answer(Feature::ClipboardRich, reply, |s| {
            s.clipboard_text = None;
            s.clipboard_files = None;
            s.clipboard_image = Some(image);
            Ok(())
        })
    }

    fn clipboard_read_files(&self, reply: Reply<Option<Vec<PathBuf>>>) {
        self.answer(Feature::ClipboardRich, reply, |s| Ok(s.clipboard_files.clone()))
    }

    fn clipboard_write_files(&self, files: Vec<PathBuf>, reply: Reply<()>) {
        self.answer(Feature::ClipboardRich, reply, |s| {
            s.clipboard_text = None;
            s.clipboard_image = None;
            s.clipboard_files = Some(files);
            Ok(())
        })
    }

    fn notify(&self, id: NotificationId, notification: Notification, reply: Reply<()>) {
        self.answer(Feature::Notifications, reply, |s| {
            s.calls.push(Call::Notify(id, notification));
            Ok(())
        })
    }

    fn recent_add(&self, path: PathBuf, reply: Reply<()>) {
        self.answer(Feature::RecentDocuments, reply, |s| {
            s.recent.retain(|p| *p != path);
            s.recent.insert(0, path);
            Ok(())
        })
    }

    fn recent_clear(&self, reply: Reply<()>) {
        self.answer(Feature::RecentDocuments, reply, |s| {
            s.recent.clear();
            Ok(())
        })
    }

    fn credential_set(&self, service: String, account: String, secret: String, reply: Reply<()>) {
        self.answer(Feature::Credentials, reply, |s| {
            s.credentials.insert((service, account), secret);
            Ok(())
        })
    }

    fn credential_get(&self, service: String, account: String, reply: Reply<Option<String>>) {
        self.answer(Feature::Credentials, reply, |s| Ok(s.credentials.get(&(service, account)).cloned()))
    }

    fn credential_delete(&self, service: String, account: String, reply: Reply<()>) {
        self.answer(Feature::Credentials, reply, |s| {
            s.credentials.remove(&(service, account));
            Ok(())
        })
    }

    fn open_url(&self, url: &str) -> Result<()> {
        self.record(Feature::Shell, Call::OpenUrl(url.into()))
    }

    fn open_path(&self, path: PathBuf) -> Result<()> {
        self.record(Feature::Shell, Call::OpenPath(path))
    }

    fn reveal_path(&self, path: PathBuf) -> Result<()> {
        self.record(Feature::Shell, Call::RevealPath(path))
    }

    fn tray_set(&self, tray: Tray, reply: Reply<()>) {
        self.answer(Feature::Tray, reply, |s| {
            s.trays.insert(tray.id.0.clone(), tray);
            Ok(())
        })
    }

    fn tray_remove(&self, id: TrayId, reply: Reply<()>) {
        self.answer(Feature::Tray, reply, |s| s.trays.remove(&id.0).map(drop).ok_or(Error::NotFound))
    }

    fn media_set(&self, now: Option<NowPlaying>, reply: Reply<()>) {
        self.answer(Feature::MediaSession, reply, |s| {
            s.now_playing = now;
            Ok(())
        })
    }

    fn audio_play(&self, id: PlayerId, options: PlayOptions, reply: Reply<()>) {
        let volume = options.volume;
        self.answer(Feature::Audio, reply, |s| {
            s.players
                .insert(id.0, FakePlayer { options, state: AudioState::Playing, position: Duration::ZERO, volume });
            Ok(())
        });
        self.emit(SystemEvent::Audio {
            player: id,
            event: AudioEvent::State {
                state: AudioState::Playing,
                position: Duration::ZERO,
                duration: None,
                error: None,
            },
        });
    }

    fn audio_control(&self, id: PlayerId, control: AudioControl, reply: Reply<()>) {
        let mut transition = None;
        self.answer(Feature::Audio, reply, |s| {
            s.calls.push(Call::AudioControl(id, control));
            let player = s.players.get_mut(&id.0).ok_or(Error::NotFound)?;
            let to = match control {
                AudioControl::Pause => Some(AudioState::Paused),
                AudioControl::Resume => Some(AudioState::Playing),
                AudioControl::Stop => Some(AudioState::Stopped),
                AudioControl::Seek(at) => {
                    player.position = at;
                    None
                }
                AudioControl::SetVolume(v) => {
                    player.volume = v;
                    None
                }
            };
            if let Some(to) = to.filter(|to| *to != player.state) {
                player.state = to;
                transition =
                    Some(AudioEvent::State { state: to, position: player.position, duration: None, error: None });
            }
            if control == AudioControl::Stop {
                s.players.remove(&id.0);
            }
            Ok(())
        });
        if let Some(event) = transition {
            self.emit(SystemEvent::Audio { player: id, event });
        }
    }
}

impl FakeBackend {
    fn record(&self, feature: Feature, call: Call) -> Result<()> {
        let mut state = self.state();
        if state.unsupported.contains(&feature) {
            return Err(Error::unsupported(feature));
        }
        state.calls.push(call);
        Ok(())
    }
}
