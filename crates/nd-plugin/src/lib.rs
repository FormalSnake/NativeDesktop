//! Plugins (ARCHITECTURE.md 3.4): a crate implements [`Plugin`], the app registers it on a
//! [`PluginHost`], and nothing in the framework is forked.
//!
//! A plugin owns one `[plugins.<name>]` config section. The host splits every `plugins` table
//! off the app's config before the app parses it, so an app keeps `deny_unknown_fields` on its
//! root struct, and parses each section into the plugin's own [`Plugin::Config`]. A section
//! that fails to parse disables that plugin and appears in the [`LoadReport`]; it never fails
//! the app.
//!
//! This crate does not name GPUI. App state a plugin needs to observe or drive (now playing,
//! a command channel) is published by the app into [`Services`] and fetched by type.

mod context;
mod host;

pub use context::{PluginContext, Services};
pub use host::{LoadReport, PluginFailure, PluginHost, split_plugins};

use serde::Serialize;
use serde::de::DeserializeOwned;

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("config section: {0}")]
    Config(#[from] toml::de::Error),
    #[error("{0}")]
    Init(String),
}

impl PluginError {
    pub fn init(msg: impl std::fmt::Display) -> Self {
        Self::Init(msg.to_string())
    }
}

pub trait Plugin: Send + 'static {
    /// The `[plugins.<name>]` table this plugin owns. Unique per app.
    type Config: DeserializeOwned + Serialize + Default + Send + 'static;

    fn name(&self) -> &'static str;

    /// One line for the settings page and the generated config docs.
    fn description(&self) -> &'static str {
        ""
    }

    /// Starts the plugin. Long-running work goes through [`PluginContext::spawn`] so the host
    /// can stop it; do not block here.
    fn init(&mut self, cx: &PluginContext, config: Self::Config) -> Result<(), PluginError>;

    /// The config file changed. The default restarts the plugin's tasks and calls
    /// [`Plugin::init`] again.
    fn config_changed(&mut self, cx: &PluginContext, config: Self::Config) -> Result<(), PluginError> {
        cx.stop_tasks();
        self.init(cx, config)
    }

    /// Called once on exit, after the plugin's spawned tasks are aborted.
    fn shutdown(&mut self) {}
}
