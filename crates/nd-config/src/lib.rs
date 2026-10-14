//! `nativedesktop.toml`: one file per app beside `Cargo.toml` (ARCHITECTURE.md 11.1). Name and
//! version come from Cargo, never from this file. Every table denies unknown keys, so a typo is an
//! error with the file path and line instead of a silently ignored setting.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The whole file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
#[schemars(title = "nativedesktop.toml")]
pub struct Config {
    pub app: App,
    pub patterns: Patterns,
    pub webview: Webview,
    pub updates: Updates,
    pub diagnostics: Diagnostics,
    pub macos: Macos,
    pub windows: Windows,
    pub linux: Linux,
    pub web: Web,
}

/// Identity: bundle id, names, icon and what the app registers with the OS.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct App {
    /// Reverse-DNS bundle id, for example `dev.formalsnake.notes`. Required.
    #[schemars(regex(pattern = r"^[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+$"))]
    pub id: String,
    /// Name shown in the Dock, window lists and installers. Defaults to the Cargo package name.
    pub display_name: Option<String>,
    /// Icon source, relative to this file. A `.icon` bundle or a 1024 px PNG.
    pub icon: Option<String>,
    /// freedesktop menu categories, for example `Office`.
    pub categories: Vec<String>,
    /// URL schemes the app opens, without the `://`.
    pub url_schemes: Vec<String>,
    pub file_associations: Vec<FileAssociation>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileAssociation {
    /// Extensions without the dot.
    pub ext: Vec<String>,
    #[serde(default)]
    pub role: FileRole,
    /// Display name of the document type. Defaults to the first extension in upper case.
    pub name: Option<String>,
    /// MIME type declared on Linux (`.desktop` `MimeType=` and shared-mime XML). Defaults to
    /// `application/x-<last app.id segment>-<first ext>`.
    pub mime: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FileRole {
    #[default]
    Editor,
    Viewer,
}

/// Project-wide pattern choices. Code can override them per app, window or element.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Patterns {
    pub tooltips: Option<PatternChoice>,
    pub menus: Option<PatternChoice>,
    pub popovers: Option<PatternChoice>,
    pub context_menu: Option<PatternChoice>,
    pub settings_window: Option<SettingsWindow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum PatternChoice {
    Native,
    Drawn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SettingsWindow {
    Window,
    Sheet,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Webview {
    pub cef: Cef,
}

/// CEF is opt-in per app. Without it the binary carries no Chromium bytes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Cef {
    pub enabled: bool,
    pub style: CefStyle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CefStyle {
    /// Off-screen rendering composited into the app's own window.
    #[default]
    Osr,
    /// Windowed Chrome-style browser with extensions.
    Chrome,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Updates {
    /// Base URL of the static feed directory. No default: each app sets its own.
    pub feed: Option<String>,
    /// Channels `nd release --channel` accepts; the first is the default. Empty means `["stable"]`.
    pub channels: Vec<String>,
    /// ed25519 public key the feed signature is checked against: 32 bytes, standard base64.
    pub public_key: Option<String>,
    pub publish: Option<Publish>,
}

/// Where `nd release` uploads the feed directory.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Publish {
    /// R2, S3, MinIO or any S3-compatible store.
    S3 {
        /// Endpoint URL. Prefer `endpoint_env` when the URL carries an account id.
        endpoint: Option<String>,
        /// Environment variable holding the endpoint URL.
        endpoint_env: Option<String>,
        bucket: String,
        /// Key prefix inside the bucket, without a trailing slash.
        prefix: Option<String>,
        /// Signing region. R2 takes `auto`, the default.
        region: Option<String>,
    },
    /// One rolling GitHub release tagged `updates` holding every channel's files, uploaded with
    /// `gh`. The feed URL is then `https://github.com/<repo>/releases/download/updates`.
    Github {
        /// `owner/name`. Defaults to the repository of the current checkout.
        repo: Option<String>,
    },
    /// `rsync` destination, local or `host:path`.
    Rsync { target: String },
    /// A local directory, also what `file://` update tests read.
    Dir { path: PathBuf },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Diagnostics {
    pub retention: Retention,
    /// Environment variable holding the Sentry DSN. Bundles are always written locally.
    pub sentry_dsn_env: Option<String>,
}

/// Overrides for the retention defaults in ARCHITECTURE.md 9.6.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Retention {
    pub keep_bundles: Option<u32>,
    pub max_bundle_age_days: Option<u32>,
    pub max_total_mb: Option<u32>,
    pub keep_logs: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Macos {
    /// Deployment target, for example `14.0`.
    pub minimum_system_version: Option<String>,
    /// Codesigning identity, as `security find-identity` prints it.
    pub signing_identity: Option<String>,
    pub notarize: bool,
    /// `xcrun notarytool store-credentials` profile name, required when `notarize` is on.
    pub notary_profile: Option<String>,
    /// Developer ID Installer identity for the `.pkg` installer. Unsigned when absent.
    pub installer_identity: Option<String>,
    /// Entitlement short names, for example `network.client`.
    pub entitlements: Vec<String>,
    /// SwiftPM package directories linked into the app, relative to this file.
    pub swift_packages: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Windows {
    pub publisher: Option<String>,
    pub installer: Installer,
    /// Accent color of the installer, `#rrggbb`.
    #[schemars(regex(pattern = r"^#[0-9A-Fa-f]{6}$"))]
    pub accent: Option<String>,
    pub signing: Option<WindowsSigning>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Installer {
    /// Splash image shown by `Setup.exe`, relative to this file.
    pub splash: Option<String>,
    pub shortcuts: Vec<Shortcut>,
    /// Build an MSI next to `Setup.exe`.
    pub msi: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Shortcut {
    Start,
    Desktop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum WindowsSigning {
    /// Azure Artifact Signing.
    Azure,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Linux {
    pub formats: Vec<LinuxFormat>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LinuxFormat {
    Appimage,
    Deb,
    Flatpak,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Web {
    /// URL path the static bundle is served under, for example `/notes/`.
    pub base_path: Option<String>,
}

#[derive(Debug)]
pub enum Error {
    Read(PathBuf, std::io::Error),
    Parse(PathBuf, toml::de::Error),
    Invalid(PathBuf, String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(path, e) => write!(f, "{}: {e}", path.display()),
            Self::Parse(path, e) => write!(f, "{}: {e}", path.display()),
            Self::Invalid(path, why) => write!(f, "{}: {why}", path.display()),
        }
    }
}

impl std::error::Error for Error {}

impl Config {
    /// Reads and validates `nativedesktop.toml`.
    pub fn load(path: &Path) -> Result<Self, Error> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::Read(path.to_owned(), e))?;
        Self::parse(&text, path)
    }

    /// `path` only labels errors.
    pub fn parse(text: &str, path: &Path) -> Result<Self, Error> {
        let config: Self = toml::from_str(text).map_err(|e| Error::Parse(path.to_owned(), e))?;
        config.validate().map_err(|why| Error::Invalid(path.to_owned(), why))?;
        Ok(config)
    }

    /// Rules the schema's types cannot express.
    pub fn validate(&self) -> Result<(), String> {
        let id = &self.app.id;
        if id.is_empty() {
            return Err("app.id is required".into());
        }
        let well_formed = id.split('.').count() >= 2
            && id
                .split('.')
                .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        if !well_formed {
            return Err(format!("app.id `{id}` is not reverse-DNS (for example dev.example.app)"));
        }
        for (i, assoc) in self.app.file_associations.iter().enumerate() {
            if assoc.ext.is_empty() || assoc.ext.iter().any(|e| e.is_empty() || e.starts_with('.')) {
                return Err(format!("app.file_associations[{i}].ext needs extensions without a leading dot"));
            }
        }
        if let Some(scheme) = self.app.url_schemes.iter().find(|s| s.is_empty() || s.contains(':')) {
            return Err(format!("app.url_schemes entry `{scheme}` must be the bare scheme"));
        }
        if self.updates.feed.is_some() != self.updates.public_key.is_some() {
            return Err(
                "updates.feed and updates.public_key go together: a feed without a key cannot be verified".into()
            );
        }
        if let Some(key) = &self.updates.public_key {
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD.decode(key.trim());
            if bytes.map_or(true, |b| b.len() != 32) {
                return Err(format!("updates.public_key `{key}` is not a base64 ed25519 key (32 bytes)"));
            }
        }
        if let Some(bad) = self.updates.channels.iter().find(|c| !is_channel(c)) {
            return Err(format!("updates.channels entry `{bad}` must be lowercase letters, digits or -"));
        }
        if self.macos.notarize && self.macos.notary_profile.is_none() {
            return Err(
                "macos.notarize needs macos.notary_profile (an `xcrun notarytool store-credentials` profile)".into()
            );
        }
        if self.macos.notarize && self.macos.signing_identity.is_none() {
            return Err(
                "macos.notarize needs macos.signing_identity: Apple only notarizes Developer ID signed code".into()
            );
        }
        Ok(())
    }
}

fn is_channel(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

impl Config {
    /// Name shown to users: `app.display_name`, else the Cargo package name.
    pub fn display_name<'a>(&'a self, package: &'a str) -> &'a str {
        self.app.display_name.as_deref().unwrap_or(package)
    }
}

impl Updates {
    /// The configured channels, `["stable"]` when none are listed.
    pub fn channels(&self) -> Vec<String> {
        if self.channels.is_empty() { vec!["stable".into()] } else { self.channels.clone() }
    }
}

impl FileAssociation {
    /// Display name of the document type.
    pub fn type_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.ext.first().map(|e| e.to_uppercase()).unwrap_or_default())
    }

    /// MIME type declared on Linux.
    pub fn mime_type(&self, app_id: &str) -> String {
        self.mime.clone().unwrap_or_else(|| {
            let app = app_id.rsplit('.').next().unwrap_or(app_id).to_lowercase();
            format!("application/x-{app}-{}", self.ext.first().map(String::as_str).unwrap_or("file"))
        })
    }
}

/// The JSON Schema editors read through `#:schema`.
pub fn schema() -> schemars::Schema {
    schemars::schema_for!(Config)
}

pub fn schema_json() -> String {
    let mut text = serde_json::to_string_pretty(&schema()).expect("schema serializes");
    text.push('\n');
    text
}
