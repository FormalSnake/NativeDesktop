//! `nd package [mac|linux|windows]`: installers from `nativedesktop.toml`, through Velopack
//! (ARCHITECTURE.md 11.2). Name and version come from `Cargo.toml`.

pub mod icon;
pub mod linux;
pub mod mac;
pub mod windows;

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Args, ValueEnum};

use crate::project::{Project, fresh_dir};

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Os {
    Mac,
    Linux,
    Windows,
}

impl Os {
    pub fn host() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Mac => "mac",
            Self::Linux => "linux",
            Self::Windows => "windows",
        }
    }
}

#[derive(Args, Debug, Clone)]
pub struct PackageArgs {
    /// Target OS; defaults to this one.
    #[arg(value_enum)]
    pub os: Option<Os>,
    /// App directory (the one holding Cargo.toml and nativedesktop.toml).
    #[arg(long, default_value = ".")]
    pub dir: PathBuf,
    /// Release channel; defaults to the first of `updates.channels`.
    #[arg(long)]
    pub channel: Option<String>,
    /// Markdown release notes, shown in the app's "what's new".
    #[arg(long)]
    pub notes: Option<PathBuf>,
    /// Package this prebuilt binary instead of running `cargo build --release` (cross-packing).
    #[arg(long)]
    pub binary: Option<PathBuf>,
    /// Velopack runtime id for cross-packing, for example `linux-arm64`.
    #[arg(long)]
    pub runtime: Option<String>,
}

/// What a packer needs beyond the project.
#[derive(Debug, Clone)]
pub struct Options {
    pub channel: String,
    /// Velopack's output directory: packages, deltas, `releases.<channel>.json`, installers.
    pub releases: PathBuf,
    pub notes: Option<PathBuf>,
    pub binary: Option<PathBuf>,
    pub runtime: Option<String>,
}

#[cfg(test)]
impl Options {
    pub fn for_test(channel: &str) -> Self {
        Self { channel: channel.into(), releases: "out".into(), notes: None, binary: None, runtime: None }
    }
}

#[derive(Debug)]
pub struct Packed {
    /// The bundle or staged directory.
    pub app: PathBuf,
    /// `.dmg`, AppImage or `Setup.exe`.
    pub installer: Option<PathBuf>,
    pub releases: PathBuf,
}

pub fn pack(project: &Project, os: Os, opts: &Options) -> Result<Packed> {
    match os {
        Os::Mac => mac::package(project, opts),
        Os::Linux => linux::package(project, opts),
        Os::Windows => windows::package(project, opts),
    }
}

pub fn run(args: PackageArgs) -> Result<()> {
    let project = Project::load(&args.dir)?;
    let os = args.os.unwrap_or_else(Os::host);
    let channel = crate::release::velopack_channel(os, &project.channel(args.channel.as_deref())?);
    // A throwaway Velopack output: `nd release` keeps its own, so packaging twice never pollutes
    // a feed.
    let releases = project.nd_dir().join("package").join(os.name()).join("velopack");
    fresh_dir(&releases)?;
    let opts = Options { channel, releases, notes: args.notes, binary: args.binary, runtime: args.runtime };
    let packed = pack(&project, os, &opts)?;
    println!("app: {}", packed.app.display());
    if let Some(installer) = &packed.installer {
        println!("installer: {}", installer.display());
    }
    println!("velopack: {}", packed.releases.display());
    Ok(())
}

/// Lists the files in a directory, sorted, for messages and tests.
pub fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;

    use super::*;

    thread_local! {
        static DIRS: RefCell<Vec<tempfile::TempDir>> = const { RefCell::new(Vec::new()) };
    }

    /// A project named `demo` 0.1.0 with `config` as its `nativedesktop.toml`, rooted in a temp
    /// directory that lives as long as the test's thread.
    pub fn project(config: &str) -> Project {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_owned();
        DIRS.with(|d| d.borrow_mut().push(dir));
        Project {
            target_dir: root.join("target"),
            root,
            name: "demo".into(),
            version: "0.1.0".into(),
            bin: "demo".into(),
            config: nd_config::Config::parse(config, Path::new("nativedesktop.toml")).unwrap(),
        }
    }
}
