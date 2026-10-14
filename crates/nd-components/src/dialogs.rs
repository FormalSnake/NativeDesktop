//! Alerts and file pickers. Each backend shows the platform's own (`NSAlert`, `NSOpenPanel` on
//! macOS; task dialogs and `IFileDialog` on Windows; the portal on Linux); results come back
//! through a callback run outside every native callback (ARCHITECTURE.md 3.13).

use std::path::PathBuf;

use nd_gpui::gpui::SharedString;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AlertStyle {
    #[default]
    Informational,
    Warning,
    Critical,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ButtonRole {
    #[default]
    Default,
    Cancel,
    Destructive,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AlertButton {
    pub label: SharedString,
    pub role: ButtonRole,
}

/// An alert. The first button is the default; with none, the platform's OK is shown.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Alert {
    pub title: SharedString,
    pub message: Option<SharedString>,
    pub style: AlertStyle,
    pub buttons: Vec<AlertButton>,
}

impl Alert {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self { title: title.into(), ..Self::default() }
    }

    pub fn message(mut self, message: impl Into<SharedString>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn style(mut self, style: AlertStyle) -> Self {
        self.style = style;
        self
    }

    pub fn button(mut self, label: impl Into<SharedString>) -> Self {
        self.buttons.push(AlertButton { label: label.into(), role: ButtonRole::Default });
        self
    }

    pub fn cancel(mut self, label: impl Into<SharedString>) -> Self {
        self.buttons.push(AlertButton { label: label.into(), role: ButtonRole::Cancel });
        self
    }

    pub fn destructive(mut self, label: impl Into<SharedString>) -> Self {
        self.buttons.push(AlertButton { label: label.into(), role: ButtonRole::Destructive });
        self
    }
}

/// The button the user picked, by its index in [`Alert::buttons`] (0 when the alert had none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlertResponse(pub usize);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpenDialog {
    pub title: Option<SharedString>,
    pub prompt: Option<SharedString>,
    pub directory: Option<PathBuf>,
    pub files: bool,
    pub directories: bool,
    pub multiple: bool,
    /// File extensions without the dot; empty allows any.
    pub extensions: Vec<SharedString>,
}

impl OpenDialog {
    /// Picks one file.
    pub fn file() -> Self {
        Self { files: true, ..Self::default() }
    }

    /// Picks one directory.
    #[nd_macros::unavailable(web)]
    pub fn directory() -> Self {
        Self { directories: true, ..Self::default() }
    }

    pub fn multiple(mut self) -> Self {
        self.multiple = true;
        self
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn prompt(mut self, prompt: impl Into<SharedString>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }

    pub fn starting_in(mut self, dir: impl Into<PathBuf>) -> Self {
        self.directory = Some(dir.into());
        self
    }

    pub fn extensions(mut self, exts: impl IntoIterator<Item = impl Into<SharedString>>) -> Self {
        self.extensions = exts.into_iter().map(Into::into).collect();
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SaveDialog {
    pub title: Option<SharedString>,
    pub prompt: Option<SharedString>,
    pub directory: Option<PathBuf>,
    pub file_name: Option<SharedString>,
    pub extensions: Vec<SharedString>,
}

impl SaveDialog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn file_name(mut self, name: impl Into<SharedString>) -> Self {
        self.file_name = Some(name.into());
        self
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn starting_in(mut self, dir: impl Into<PathBuf>) -> Self {
        self.directory = Some(dir.into());
        self
    }

    pub fn extensions(mut self, exts: impl IntoIterator<Item = impl Into<SharedString>>) -> Self {
        self.extensions = exts.into_iter().map(Into::into).collect();
        self
    }
}
