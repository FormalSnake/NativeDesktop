//! The in-app updater: Velopack's `UpdateManager` over [`VerifiedSource`].

use std::path::{Path, PathBuf};
#[cfg(not(windows))]
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use velopack::{UpdateCheck, UpdateInfo, UpdateManager, UpdateOptions, VelopackApp, VelopackAsset};

use crate::feed::PublicKey;
use crate::source::{Location, VerifiedSource};
use crate::{Unavailable, UpdateConfig};

/// Velopack's startup hook. Runs install/uninstall hooks (and exits for them) and applies a
/// downloaded update whose apply-on-quit did not run. Call it before anything else in `main`;
/// `nd::Application::run` does.
pub fn startup() {
    if crate::compiled_out().is_some() {
        return;
    }
    VelopackApp::build().run();
}

/// A newer release on the app's channel.
#[derive(Clone, Debug)]
pub struct Update {
    pub version: String,
    pub notes_markdown: String,
    pub notes_html: String,
    /// Velopack will try deltas from the installed version before the full package.
    pub delta: bool,
    info: UpdateInfo,
}

/// Release notes of the version the app just updated to, shown once.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhatsNew {
    pub version: String,
    pub notes_markdown: String,
    pub notes_html: String,
}

#[derive(Debug)]
pub struct Error(pub velopack::Error);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "update: {}", self.0)
    }
}

impl std::error::Error for Error {}

impl From<velopack::Error> for Error {
    fn from(e: velopack::Error) -> Self {
        Self(e)
    }
}

pub struct Updater {
    manager: UpdateManager,
    state_dir: PathBuf,
}

impl Updater {
    /// `state_dir` holds the "what's new" record (the app's data dir plus `updates`). `channel`
    /// (`beta`) switches away from the channel the app was installed from; `None` keeps it.
    pub fn new(config: &UpdateConfig, state_dir: &Path, channel: Option<&str>) -> Result<Self, Unavailable> {
        if let Some(reason) = crate::compiled_out() {
            return Err(reason);
        }
        let key = PublicKey::parse(config.public_key).map_err(|e| Unavailable::Misconfigured(e.to_string()))?;
        let source = VerifiedSource::new(Location::parse(config.feed), key);
        let options = UpdateOptions {
            ExplicitChannel: channel.map(crate::velopack_channel),
            AllowVersionDowngrade: false,
            MaximumDeltasBeforeFallback: 0,
        };
        let manager = UpdateManager::new(source, Some(options), None).map_err(|e| match e {
            velopack::Error::NotInstalled(why) => Unavailable::NotInstalled(why),
            other => Unavailable::Misconfigured(other.to_string()),
        })?;
        Ok(Self { manager, state_dir: state_dir.to_owned() })
    }

    pub fn current_version(&self) -> String {
        self.manager.get_current_version_as_string()
    }

    /// Fetches and verifies the feed. `Ok(None)` when the app is current.
    pub fn check(&self) -> Result<Option<Update>, Error> {
        Ok(match self.manager.check_for_updates()? {
            UpdateCheck::UpdateAvailable(info) => Some(Update {
                version: info.TargetFullRelease.Version.clone(),
                notes_markdown: info.TargetFullRelease.NotesMarkdown.clone(),
                notes_html: info.TargetFullRelease.NotesHtml.clone(),
                delta: info.BaseRelease.is_some() && !info.DeltasToTarget.is_empty(),
                info: *info,
            }),
            UpdateCheck::NoUpdateAvailable | UpdateCheck::RemoteIsEmpty => None,
        })
    }

    /// Downloads (deltas first) into Velopack's packages directory and records the notes for
    /// [`whats_new`].
    pub fn download(&self, update: &Update) -> Result<(), Error> {
        self.manager.download_updates(&update.info, None)?;
        let record = WhatsNew {
            version: update.version.clone(),
            notes_markdown: update.notes_markdown.clone(),
            notes_html: update.notes_html.clone(),
        };
        if std::fs::create_dir_all(&self.state_dir).is_ok() {
            let _ = std::fs::write(pending_path(&self.state_dir), serde_json::to_vec(&record).unwrap_or_default());
        }
        Ok(())
    }

    /// Applies `update` after the process exits, without relaunching. A process that never
    /// reaches `exit` gets it on its next launch through [`startup`] instead.
    pub fn apply_on_quit(&self, update: &Update) {
        arm_quit_hook(self.manager.clone(), update.info.TargetFullRelease.clone());
    }

    /// Quits now, applies, relaunches.
    pub fn apply_and_restart(&self, update: &Update) -> Result<(), Error> {
        Ok(self.manager.apply_updates_and_restart(&update.info)?)
    }

    /// Check, download and arm apply-on-quit after `first_after`, then every `every`, on a
    /// background thread. `on_ready` runs there once an update is downloaded; `on_error` for a
    /// failed check, a refused feed included.
    pub fn spawn_background(
        self,
        first_after: Duration,
        every: Duration,
        on_ready: impl Fn(&Update) + Send + 'static,
        on_error: impl Fn(&Error) + Send + 'static,
    ) -> std::thread::JoinHandle<()> {
        std::thread::Builder::new()
            .name("nd-update".into())
            .spawn(move || {
                std::thread::sleep(first_after);
                loop {
                    match self.check().and_then(|u| u.map(|u| self.download(&u).map(|()| u)).transpose()) {
                        Ok(Some(update)) => {
                            log::info!("update {} downloaded (delta: {})", update.version, update.delta);
                            self.apply_on_quit(&update);
                            on_ready(&update);
                            return;
                        }
                        Ok(None) => log::debug!("no update"),
                        Err(e) => {
                            log::warn!("{e}");
                            on_error(&e);
                        }
                    }
                    std::thread::sleep(every);
                }
            })
            .expect("spawn the update thread")
    }
}

fn pending_path(state_dir: &Path) -> PathBuf {
    state_dir.join("pending.json")
}

fn seen_path(state_dir: &Path) -> PathBuf {
    state_dir.join("whats-new-seen")
}

/// The notes of the update that produced `current_version`, until [`mark_whats_new_seen`].
pub fn whats_new(state_dir: &Path, current_version: &str) -> Option<WhatsNew> {
    let record: WhatsNew = serde_json::from_slice(&std::fs::read(pending_path(state_dir)).ok()?).ok()?;
    let seen = std::fs::read_to_string(seen_path(state_dir)).unwrap_or_default();
    (record.version == current_version && seen.trim() != current_version).then_some(record)
}

pub fn mark_whats_new_seen(state_dir: &Path, current_version: &str) {
    let _ = std::fs::create_dir_all(state_dir);
    let _ = std::fs::write(seen_path(state_dir), current_version);
}

#[cfg(not(windows))]
static ON_QUIT: OnceLock<Mutex<Option<(UpdateManager, VelopackAsset)>>> = OnceLock::new();

/// Unix: an `atexit` hook starts the updater as the process exits. Windows: the process ends
/// through `ExitProcess`, which runs no C `atexit` handlers, so Velopack's updater starts now and
/// waits for this pid to exit, however it exits, before it swaps the install.
fn arm_quit_hook(manager: UpdateManager, asset: VelopackAsset) {
    #[cfg(windows)]
    {
        if let Err(e) = manager.wait_exit_then_apply_updates(&asset, true, false, Vec::<String>::new()) {
            log::warn!("arming the update for quit: {e}");
        }
    }
    #[cfg(not(windows))]
    {
        let slot = ON_QUIT.get_or_init(|| {
            #[cfg(unix)]
            // SAFETY: `apply_at_exit` is a plain `extern "C" fn()` that never unwinds.
            unsafe {
                libc::atexit(apply_at_exit);
            }
            Mutex::new(None)
        });
        if let Ok(mut slot) = slot.lock() {
            *slot = Some((manager, asset));
        }
    }
}

#[cfg(unix)]
extern "C" fn apply_at_exit() {
    let _ = std::panic::catch_unwind(|| {
        let armed = ON_QUIT.get().and_then(|slot| slot.try_lock().ok()?.take());
        if let Some((manager, asset)) = armed {
            // The updater waits for this pid to exit, then swaps the bundle in place.
            let _ = manager.wait_exit_then_apply_updates(&asset, true, false, Vec::<String>::new());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whats_new_shows_once_for_the_version_it_was_recorded_for() {
        let dir = tempfile::tempdir().unwrap();
        let record = WhatsNew { version: "0.1.1".into(), notes_markdown: "Faster".into(), notes_html: String::new() };
        std::fs::write(pending_path(dir.path()), serde_json::to_vec(&record).unwrap()).unwrap();
        assert_eq!(whats_new(dir.path(), "0.1.0"), None);
        assert_eq!(whats_new(dir.path(), "0.1.1"), Some(record));
        mark_whats_new_seen(dir.path(), "0.1.1");
        assert_eq!(whats_new(dir.path(), "0.1.1"), None);
    }

    #[test]
    fn an_app_outside_a_velopack_bundle_is_not_installed() {
        let key = crate::feed::SecretKey::generate().public_key().to_base64();
        let key: &'static str = Box::leak(key.into_boxed_str());
        let config = UpdateConfig { feed: "/nonexistent", public_key: key };
        let dir = tempfile::tempdir().unwrap();
        match Updater::new(&config, dir.path(), None) {
            Err(Unavailable::NotInstalled(_)) => {}
            Err(other) => panic!("expected NotInstalled, got {other}"),
            Ok(_) => panic!("a test binary is not a Velopack install"),
        }
    }
}
