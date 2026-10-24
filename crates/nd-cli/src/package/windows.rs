//! Windows: a branded Velopack `Setup.exe` (name, icon, splash, publisher, accent and shortcuts
//! from `[windows]`), an MSI on request, Azure Artifact Signing when configured. `files` are
//! copied beside the executable.
//!
//! Signing with Azure runs only on a Windows host (`vpk` signs through signtool there); a cross
//! pack from macOS or Linux signs only through `ND_WINDOWS_SIGN_TEMPLATE`, a command with
//! `{{file}}` (for example jsign), and otherwise refuses rather than ship unsigned by accident.
//! File associations and URL schemes are not registered by Setup.exe yet.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use nd_config::{Shortcut, WindowsSigning};

use super::{Options, Packed, icon};
use crate::project::{Project, fresh_dir};
use crate::vpk;

pub fn package(project: &Project, opts: &Options) -> Result<Packed> {
    if super::cef::enabled(project) {
        bail!("[webview] engine = \"cef\": bundling Chromium for Windows is not implemented yet (macOS only)");
    }
    let work = project.nd_dir().join("package/windows");
    fresh_dir(&work)?;
    let exe = match &opts.binary {
        Some(path) => path.clone(),
        None if cfg!(windows) => project.build_release(None)?,
        None => bail!("nd package windows off Windows needs --binary <a Windows build of {}.exe>", project.bin),
    };
    let dir = work.join("app");
    std::fs::create_dir_all(&dir)?;
    std::fs::copy(&exe, dir.join(format!("{}.exe", project.bin)))
        .with_context(|| format!("copying {}", exe.display()))?;
    copy_files(project, &dir)?;
    let ico = match &project.config.app.icon {
        Some(source) => {
            let path = work.join("app.ico");
            icon::write_ico(&icon::load(&project.resolve(source))?, &path)?;
            Some(path)
        }
        None => None,
    };
    let signing = Signing::from_env(project, cfg!(windows))?;
    let vpk = vpk::locate()?;
    std::fs::create_dir_all(&opts.releases)?;
    vpk::run(&vpk, &pack_args(project, opts, &dir, ico.as_deref(), &signing, cfg!(windows)))?;
    let setup = opts.releases.join(format!("{}-{}-Setup.exe", project.id(), opts.channel));
    Ok(Packed { app: dir, installer: setup.exists().then_some(setup), releases: opts.releases.clone() })
}

fn copy_files(project: &Project, dir: &Path) -> Result<()> {
    for file in &project.config.windows.files {
        let source = project.resolve(file);
        let name = source.file_name().with_context(|| format!("windows.files: `{file}` names no file"))?;
        std::fs::copy(&source, dir.join(name))
            .with_context(|| format!("windows.files: copying {}", source.display()))?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signing {
    None,
    /// `metadata.json` for Azure Artifact Signing (`ND_AZURE_SIGN_METADATA`).
    Azure(PathBuf),
    /// A signing command with `{{file}}` (`ND_WINDOWS_SIGN_TEMPLATE`).
    Template(String),
}

impl Signing {
    fn from_env(project: &Project, on_windows: bool) -> Result<Self> {
        let template = std::env::var("ND_WINDOWS_SIGN_TEMPLATE").ok().filter(|t| !t.is_empty());
        match project.config.windows.signing {
            None => Ok(Self::None),
            Some(WindowsSigning::Azure) if on_windows => {
                let metadata = std::env::var_os("ND_AZURE_SIGN_METADATA")
                    .context("windows.signing = \"azure\" needs ND_AZURE_SIGN_METADATA (path to metadata.json)")?;
                Ok(Self::Azure(metadata.into()))
            }
            Some(WindowsSigning::Azure) => template.map(Self::Template).context(
                "windows.signing = \"azure\" signs on a Windows host; cross-packing needs ND_WINDOWS_SIGN_TEMPLATE",
            ),
        }
    }
}

pub fn pack_args(
    project: &Project,
    opts: &Options,
    dir: &Path,
    ico: Option<&Path>,
    signing: &Signing,
    on_windows: bool,
) -> Vec<String> {
    let win = &project.config.windows;
    let mut args: Vec<String> = Vec::new();
    if !on_windows {
        args.push("[win]".into());
    }
    args.extend([
        "pack".into(),
        "--runtime".into(),
        opts.runtime.clone().unwrap_or_else(|| "win-x64".into()),
        "--packId".into(),
        project.id().into(),
        "--packVersion".into(),
        project.version.clone(),
        "--packDir".into(),
        dir.display().to_string(),
        "--mainExe".into(),
        format!("{}.exe", project.bin),
        "--packTitle".into(),
        project.display_name().into(),
        "--channel".into(),
        opts.channel.clone(),
        "--outputDir".into(),
        opts.releases.display().to_string(),
    ]);
    if let Some(publisher) = &win.publisher {
        args.extend(["--packAuthors".into(), publisher.clone()]);
    }
    if let Some(ico) = ico {
        args.extend(["--icon".into(), ico.display().to_string()]);
    }
    if let Some(splash) = &win.installer.splash {
        args.extend(["--splashImage".into(), project.resolve(splash).display().to_string()]);
    }
    if let Some(accent) = &win.accent {
        args.extend(["--splashProgressColor".into(), accent.clone()]);
    }
    if !win.installer.shortcuts.is_empty() {
        let locations: Vec<&str> = win
            .installer
            .shortcuts
            .iter()
            .map(|s| match s {
                Shortcut::Start => "StartMenuRoot",
                Shortcut::Desktop => "Desktop",
            })
            .collect();
        args.extend(["--shortcuts".into(), locations.join(",")]);
    }
    if win.installer.msi && on_windows {
        args.extend(["--msi".into(), "--instLocation".into(), "PerUser".into()]);
    }
    match signing {
        Signing::None => {}
        Signing::Azure(metadata) => args.extend(["--azureTrustedSignFile".into(), metadata.display().to_string()]),
        Signing::Template(template) => args.extend(["--signTemplate".into(), template.clone()]),
    }
    if let Some(notes) = &opts.notes {
        args.extend(["--releaseNotes".into(), notes.display().to_string()]);
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::tests::project;

    const BRANDED: &str = r##"
[app]
id = "dev.formalsnake.notes"
display_name = "Notes"
[windows]
publisher = "FormalSnake"
accent = "#3366ff"
installer = { splash = "assets/splash.png", shortcuts = ["start", "desktop"], msi = true }
signing = "azure"
"##;

    #[test]
    fn setup_is_branded_from_the_config() {
        let p = project(BRANDED);
        let opts = Options::for_test("stable");
        let args = pack_args(&p, &opts, Path::new("app"), Some(Path::new("app.ico")), &Signing::None, false);
        let joined = args.join(" ");
        assert!(joined.starts_with("[win] pack --runtime win-x64 --packId dev.formalsnake.notes"), "{joined}");
        for needle in [
            "--mainExe demo.exe",
            "--packTitle Notes",
            "--packAuthors FormalSnake",
            "--icon app.ico",
            "--splashProgressColor #3366ff",
            "--shortcuts StartMenuRoot,Desktop",
        ] {
            assert!(joined.contains(needle), "missing {needle} in {joined}");
        }
        assert!(joined.contains("assets/splash.png"));
        assert!(!joined.contains("--msi"), "MSI only builds on a Windows host");

        let native = pack_args(&p, &opts, Path::new("app"), None, &Signing::Azure("m.json".into()), true).join(" ");
        assert!(native.starts_with("pack "));
        assert!(native.contains("--msi --instLocation PerUser"));
        assert!(native.contains("--azureTrustedSignFile m.json"));
    }

    #[test]
    fn files_land_beside_the_executable() {
        let p = project("[app]\nid = \"a.b\"\n[windows]\nfiles = [\"bin/helper.exe\"]\n");
        std::fs::create_dir_all(p.root.join("bin")).unwrap();
        std::fs::write(p.root.join("bin/helper.exe"), b"helper").unwrap();
        let dir = p.root.join("app");
        std::fs::create_dir_all(&dir).unwrap();
        copy_files(&p, &dir).unwrap();
        assert_eq!(std::fs::read(dir.join("helper.exe")).unwrap(), b"helper");

        let missing = project("[app]\nid = \"a.b\"\n[windows]\nfiles = [\"gone.exe\"]\n");
        assert!(copy_files(&missing, &missing.root).unwrap_err().to_string().contains("gone.exe"));
    }

    #[test]
    fn azure_signing_off_windows_needs_a_template() {
        let p = project(BRANDED);
        // SAFETY: tests in this module do not read the variable concurrently.
        unsafe { std::env::remove_var("ND_WINDOWS_SIGN_TEMPLATE") };
        assert!(Signing::from_env(&p, false).unwrap_err().to_string().contains("ND_WINDOWS_SIGN_TEMPLATE"));
        assert_eq!(Signing::from_env(&project("[app]\nid = \"a.b\"\n"), false).unwrap(), Signing::None);
    }
}
