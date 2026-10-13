//! Discord Rich Presence as an nd plugin.
//!
//! Written against `nd-plugin` and `nd-ipc` only, the way a crate outside this repo would be.
//! The app registers [`DiscordPlugin`] and publishes what it is doing through a
//! [`PresenceFeed`] in its [`nd_plugin::Services`]; users turn it on and set their application
//! id under `[plugins.discord]`. Discord not running is the normal case, not an error: the
//! plugin keeps looking on nd-ipc's reconnect ladder and stays out of the app's way.

mod session;
mod wire;

use std::path::PathBuf;
use std::time::Instant;

use nd_ipc::ladder::{Ladder, LadderConfig};
use nd_plugin::{Plugin, PluginContext, PluginError};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

pub use session::{Activity, now_unix};

/// `[plugins.discord]`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct DiscordConfig {
    pub enabled: bool,
    /// The Discord application id (developer portal). Empty keeps the plugin idle.
    pub client_id: String,
    pub show_elapsed: bool,
    /// Asset key used when the activity names none.
    pub default_image: Option<String>,
    /// Look for `discord-ipc-N` here instead of the usual runtime and temp dirs.
    pub socket_dir: Option<PathBuf>,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self { enabled: true, client_id: String::new(), show_elapsed: true, default_image: None, socket_dir: None }
    }
}

/// Published by the app: the activity to show, `None` for nothing.
pub struct PresenceFeed(pub watch::Receiver<Option<Activity>>);

/// Published by the plugin so the app can show a status line.
pub struct DiscordStatusFeed(pub watch::Receiver<DiscordStatus>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscordStatus {
    /// Disabled in config, or no `client_id`.
    Idle,
    /// Looking for a running Discord.
    Searching,
    Connected,
}

#[derive(Default)]
pub struct DiscordPlugin {
    ladder: Option<LadderConfig>,
}

impl DiscordPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    /// Tighter backoff for tests.
    pub fn with_ladder(ladder: LadderConfig) -> Self {
        Self { ladder: Some(ladder) }
    }
}

impl Plugin for DiscordPlugin {
    type Config = DiscordConfig;

    fn name(&self) -> &'static str {
        "discord"
    }

    fn description(&self) -> &'static str {
        "Show what the app is doing as Discord Rich Presence."
    }

    fn init(&mut self, cx: &PluginContext, config: DiscordConfig) -> Result<(), PluginError> {
        let (status_tx, status_rx) = watch::channel(DiscordStatus::Idle);
        cx.services().insert(DiscordStatusFeed(status_rx));
        if !config.enabled || config.client_id.trim().is_empty() {
            tracing::debug!("discord presence idle: disabled or no client_id");
            return Ok(());
        }
        let Some(feed) = cx.services().get::<PresenceFeed>().map(|f| f.0.clone()) else {
            return Err(PluginError::init("the app did not publish a PresenceFeed"));
        };
        let ladder = Ladder::new(self.ladder.unwrap_or_default());
        cx.spawn(connection_loop(config, feed, status_tx, ladder));
        Ok(())
    }
}

async fn connection_loop(
    config: DiscordConfig,
    feed: watch::Receiver<Option<Activity>>,
    status: watch::Sender<DiscordStatus>,
    mut ladder: Ladder,
) {
    let settings = session::Settings {
        client_id: config.client_id.trim().to_owned(),
        show_elapsed: config.show_elapsed,
        default_image: config.default_image.clone(),
    };
    loop {
        let _ = status.send(DiscordStatus::Searching);
        if let Some(stream) = connect_any(config.socket_dir.as_deref()).await {
            ladder.note_connected(Instant::now());
            let ready = status.clone();
            let outcome = session::run(stream, &settings, feed.clone(), move || {
                let _ = ready.send(DiscordStatus::Connected);
            })
            .await;
            ladder.note_disconnected(Instant::now());
            match outcome {
                Ok(session::Ended::Rejected(why)) => tracing::warn!("discord refused the connection: {why}"),
                Ok(session::Ended::Closed) => tracing::debug!("discord closed the connection"),
                Err(e) => tracing::debug!("discord session failed: {e}"),
            }
        }
        let _ = status.send(DiscordStatus::Searching);
        tokio::time::sleep(ladder.next_delay()).await;
    }
}

async fn connect_any(dir: Option<&std::path::Path>) -> Option<nd_ipc::LocalStream> {
    for path in wire::socket_candidates(dir) {
        if let Ok(stream) = nd_ipc::connect_path(&path).await {
            return Some(stream);
        }
    }
    None
}
