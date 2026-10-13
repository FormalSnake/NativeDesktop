use std::collections::BTreeSet;

use tokio::runtime::Handle;

use crate::context::{PluginContext, Services};
use crate::{Plugin, PluginError};

/// Splits the `plugins` table off an app's config. The remainder is safe to hand to a strict
/// root struct. Returns the sections and, when `plugins` was present but not a table, a note
/// for the report.
pub fn split_plugins(root: &mut toml::Table) -> (toml::Table, Option<String>) {
    match root.remove("plugins") {
        None => (toml::Table::new(), None),
        Some(toml::Value::Table(sections)) => (sections, None),
        Some(other) => (
            toml::Table::new(),
            Some(format!("`plugins` must be a table of [plugins.<name>] sections, got {}", other.type_str())),
        ),
    }
}

#[derive(Debug)]
pub struct PluginFailure {
    pub plugin: &'static str,
    pub error: PluginError,
}

/// What a load or reload did. Nothing in here stops the app.
#[derive(Debug, Default)]
pub struct LoadReport {
    pub started: Vec<&'static str>,
    pub failed: Vec<PluginFailure>,
    /// `[plugins.<name>]` sections no registered plugin claims (usually a plugin this build
    /// does not include).
    pub unclaimed: Vec<String>,
    pub malformed_plugins_key: Option<String>,
}

trait Erased: Send {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn default_section(&self) -> toml::Value;
    fn start(&mut self, cx: &PluginContext, section: Option<&toml::Value>) -> Result<(), PluginError>;
    fn reload(&mut self, cx: &PluginContext, section: Option<&toml::Value>) -> Result<(), PluginError>;
    fn shutdown(&mut self);
}

fn parse<C: serde::de::DeserializeOwned + Default>(section: Option<&toml::Value>) -> Result<C, PluginError> {
    match section {
        Some(value) => Ok(value.clone().try_into()?),
        None => Ok(C::default()),
    }
}

impl<P: Plugin> Erased for P {
    fn name(&self) -> &'static str {
        Plugin::name(self)
    }
    fn description(&self) -> &'static str {
        Plugin::description(self)
    }
    fn default_section(&self) -> toml::Value {
        toml::Value::try_from(P::Config::default()).unwrap_or_else(|_| toml::Value::Table(toml::Table::new()))
    }
    fn start(&mut self, cx: &PluginContext, section: Option<&toml::Value>) -> Result<(), PluginError> {
        let config = parse::<P::Config>(section)?;
        self.init(cx, config)
    }
    fn reload(&mut self, cx: &PluginContext, section: Option<&toml::Value>) -> Result<(), PluginError> {
        let config = parse::<P::Config>(section)?;
        self.config_changed(cx, config)
    }
    fn shutdown(&mut self) {
        Plugin::shutdown(self);
    }
}

struct Slot {
    plugin: Box<dyn Erased>,
    cx: PluginContext,
    running: bool,
}

pub struct PluginHost {
    app: String,
    handle: Handle,
    services: Services,
    slots: Vec<Slot>,
}

impl PluginHost {
    pub fn new(app: impl Into<String>, handle: Handle, services: Services) -> Self {
        Self { app: app.into(), handle, services, slots: Vec::new() }
    }

    /// # Panics
    /// When two plugins share a name; that is a bug in the app's registration, not user input.
    pub fn register<P: Plugin>(&mut self, plugin: P) {
        let name = Plugin::name(&plugin);
        assert!(self.slots.iter().all(|s| s.plugin.name() != name), "plugin `{name}` registered twice");
        let cx = PluginContext::new(&self.app, name, self.handle.clone(), self.services.clone());
        self.slots.push(Slot { plugin: Box::new(plugin), cx, running: false });
    }

    /// Removes the `plugins` table from `root` (leaving the app's own keys) and starts every
    /// registered plugin from its section, or from its defaults when it has none.
    pub fn load(&mut self, root: &mut toml::Table) -> LoadReport {
        let (sections, malformed) = split_plugins(root);
        let mut report = LoadReport { malformed_plugins_key: malformed, ..LoadReport::default() };
        self.report_unclaimed(&sections, &mut report);
        for slot in &mut self.slots {
            if slot.running {
                continue;
            }
            match slot.plugin.start(&slot.cx, sections.get(slot.plugin.name())) {
                Ok(()) => {
                    slot.running = true;
                    report.started.push(slot.plugin.name());
                }
                Err(error) => {
                    slot.cx.stop_tasks();
                    tracing::warn!(plugin = slot.plugin.name(), "plugin not started: {error}");
                    report.failed.push(PluginFailure { plugin: slot.plugin.name(), error });
                }
            }
        }
        report
    }

    /// Applies a changed config file. A plugin whose new section does not parse keeps running
    /// on its previous config and shows up in `failed`; a plugin that failed earlier gets
    /// another start attempt.
    pub fn reload(&mut self, root: &mut toml::Table) -> LoadReport {
        let (sections, malformed) = split_plugins(root);
        let mut report = LoadReport { malformed_plugins_key: malformed, ..LoadReport::default() };
        self.report_unclaimed(&sections, &mut report);
        for slot in &mut self.slots {
            let section = sections.get(slot.plugin.name());
            let result = if slot.running {
                slot.plugin.reload(&slot.cx, section)
            } else {
                slot.plugin.start(&slot.cx, section).inspect(|()| slot.running = true)
            };
            match result {
                Ok(()) => report.started.push(slot.plugin.name()),
                Err(error) => report.failed.push(PluginFailure { plugin: slot.plugin.name(), error }),
            }
        }
        report
    }

    /// Every registered plugin's defaults as a `plugins` table: what a generated config
    /// reference, settings page or Home Manager module is built from.
    pub fn default_sections(&self) -> toml::Table {
        self.slots.iter().map(|s| (s.plugin.name().to_owned(), s.plugin.default_section())).collect()
    }

    pub fn descriptions(&self) -> Vec<(&'static str, &'static str)> {
        self.slots.iter().map(|s| (s.plugin.name(), s.plugin.description())).collect()
    }

    /// Aborts every plugin's tasks, then calls `shutdown` in reverse registration order.
    pub fn shutdown(&mut self) {
        for slot in self.slots.iter_mut().rev() {
            slot.cx.stop_tasks();
            if slot.running {
                slot.plugin.shutdown();
                slot.running = false;
            }
        }
    }

    fn report_unclaimed(&self, sections: &toml::Table, report: &mut LoadReport) {
        let known: BTreeSet<&str> = self.slots.iter().map(|s| s.plugin.name()).collect();
        report.unclaimed = sections.keys().filter(|k| !known.contains(k.as_str())).cloned().collect();
    }
}

impl Drop for PluginHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}
