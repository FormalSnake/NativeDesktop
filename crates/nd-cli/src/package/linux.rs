//! Linux: a staged `.AppDir` (binary, `.desktop` with MimeType, shared-mime XML, icons) that
//! Velopack turns into a self-updating AppImage. A pre-built AppDir keeps our `.desktop`, which
//! is the only way to get file associations and URL schemes into it.
//!
//! The AppImage's `.desktop` and MIME XML reach the system menu when an integrator such as
//! appimaged or AppImageLauncher installs it; a bare AppImage registers nothing.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use nd_config::LinuxFormat;

use super::{Options, Packed, icon};
use crate::project::{Project, fresh_dir, write, xml_escape};
use crate::vpk;

pub fn package(project: &Project, opts: &Options) -> Result<Packed> {
    if super::cef::enabled(project) {
        bail!("[webview] engine = \"cef\": bundling Chromium for Linux is not implemented yet (macOS only)");
    }
    let formats = &project.config.linux.formats;
    if let Some(other) = formats.iter().find(|f| **f != LinuxFormat::Appimage) {
        bail!("linux.formats: {other:?} is not built by nd package yet; only appimage is");
    }
    let work = project.nd_dir().join("package/linux");
    fresh_dir(&work)?;
    let exe = match &opts.binary {
        Some(path) => path.clone(),
        None if cfg!(target_os = "linux") => project.build_release(None)?,
        None => bail!("nd package linux off Linux needs --binary <a Linux build of {}>", project.bin),
    };
    let appdir = stage_appdir(project, &exe, &work)?;
    let vpk = vpk::locate()?;
    std::fs::create_dir_all(&opts.releases)?;
    vpk::run(&vpk, &pack_args(project, opts, &appdir))?;
    let appimage = opts.releases.join(format!("{}-{}.AppImage", project.id(), opts.channel));
    Ok(Packed { app: appdir, installer: appimage.exists().then_some(appimage), releases: opts.releases.clone() })
}

/// The `vpk [linux] pack` arguments; the `[linux]` directive is what lets macOS and Windows
/// hosts cross-pack.
pub fn pack_args(project: &Project, opts: &Options, appdir: &Path) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "[linux]".into(),
        "pack".into(),
        "--runtime".into(),
        opts.runtime.clone().unwrap_or_else(|| "linux-x64".into()),
        "--packId".into(),
        project.id().into(),
        "--packVersion".into(),
        project.version.clone(),
        "--packDir".into(),
        appdir.display().to_string(),
        "--mainExe".into(),
        project.bin.clone(),
        "--packTitle".into(),
        project.display_name().into(),
        "--channel".into(),
        opts.channel.clone(),
        "--outputDir".into(),
        opts.releases.display().to_string(),
    ];
    if let Some(notes) = &opts.notes {
        args.extend(["--releaseNotes".into(), notes.display().to_string()]);
    }
    args
}

fn stage_appdir(project: &Project, exe: &Path, work: &Path) -> Result<PathBuf> {
    let id = project.id();
    let appdir = work.join(format!("{}.AppDir", project.display_name()));
    let bin = appdir.join("usr/bin");
    std::fs::create_dir_all(&bin)?;
    std::fs::copy(exe, bin.join(&project.bin)).with_context(|| format!("copying {}", exe.display()))?;

    let source = project.config.app.icon.as_deref().context("app.icon is required for an AppImage (a 1024 px PNG)")?;
    let image = icon::load(&project.resolve(source))?;
    icon::write_png(&image, 512, &appdir.join(format!("{id}.png")))?;
    for size in [64u32, 128, 256, 512] {
        let dir = appdir.join(format!("usr/share/icons/hicolor/{size}x{size}/apps"));
        std::fs::create_dir_all(&dir)?;
        icon::write_png(&image, size, &dir.join(format!("{id}.png")))?;
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(format!("{id}.png"), appdir.join(".DirIcon"))?;

    let desktop = desktop_entry(project);
    write(&appdir.join(format!("{id}.desktop")), &desktop)?;
    write(&appdir.join(format!("usr/share/applications/{id}.desktop")), &desktop)?;
    if let Some(xml) = mime_xml(project) {
        write(&appdir.join(format!("usr/share/mime/packages/{id}.xml")), xml)?;
    }
    let apprun = appdir.join("AppRun");
    write(
        &apprun,
        format!(
            "#!/bin/sh\nHERE=\"$(dirname \"$(readlink -f \"$0\")\")\"\nexec \"$HERE/usr/bin/{}\" \"$@\"\n",
            project.bin
        ),
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&apprun, std::fs::Permissions::from_mode(0o755))?;
        std::fs::set_permissions(bin.join(&project.bin), std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(appdir)
}

/// The freedesktop entry, with `MimeType=` for file associations and `x-scheme-handler/*`.
pub fn desktop_entry(project: &Project) -> String {
    let app = &project.config.app;
    let mut mime: Vec<String> = app.file_associations.iter().map(|a| a.mime_type(&app.id)).collect();
    mime.extend(app.url_schemes.iter().map(|s| format!("x-scheme-handler/{s}")));
    let categories = if app.categories.is_empty() { vec!["Utility".to_string()] } else { app.categories.clone() };
    let mut out = format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec={} %U\nIcon={}\nTerminal=false\nCategories={};\nStartupWMClass={}\n",
        project.display_name(),
        project.bin,
        app.id,
        categories.join(";"),
        app.id
    );
    if !mime.is_empty() {
        out.push_str(&format!("MimeType={};\n", mime.join(";")));
    }
    out
}

/// shared-mime-info XML declaring every file association's type, `None` without any.
pub fn mime_xml(project: &Project) -> Option<String> {
    let app = &project.config.app;
    if app.file_associations.is_empty() {
        return None;
    }
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<mime-info xmlns=\"http://www.freedesktop.org/standards/shared-mime-info\">\n",
    );
    for assoc in &app.file_associations {
        out.push_str(&format!("  <mime-type type=\"{}\">\n", xml_escape(&assoc.mime_type(&app.id))));
        out.push_str(&format!("    <comment>{}</comment>\n", xml_escape(&assoc.type_name())));
        for ext in &assoc.ext {
            out.push_str(&format!("    <glob pattern=\"*.{}\"/>\n", xml_escape(ext)));
        }
        out.push_str("  </mime-type>\n");
    }
    out.push_str("</mime-info>\n");
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::tests::project;

    const NOTES: &str = r#"
[app]
id = "dev.formalsnake.notes"
display_name = "Notes"
categories = ["Office"]
url_schemes = ["notes"]
[[app.file_associations]]
ext = ["md", "markdown"]
mime = "text/markdown"
[[app.file_associations]]
ext = ["nte"]
"#;

    #[test]
    fn desktop_entry_declares_types_and_schemes() {
        let entry = desktop_entry(&project(NOTES));
        assert!(entry.contains("Exec=demo %U\n"), "{entry}");
        assert!(entry.contains("Icon=dev.formalsnake.notes\n"));
        assert!(entry.contains("Categories=Office;\n"));
        assert!(entry.contains("MimeType=text/markdown;application/x-notes-nte;x-scheme-handler/notes;\n"), "{entry}");
    }

    #[test]
    fn mime_xml_globs_every_extension() {
        let xml = mime_xml(&project(NOTES)).unwrap();
        assert!(xml.contains("<mime-type type=\"text/markdown\">"));
        assert!(xml.contains("<glob pattern=\"*.markdown\"/>"));
        assert!(xml.contains("<mime-type type=\"application/x-notes-nte\">\n    <comment>NTE</comment>"));
        assert!(mime_xml(&project("[app]\nid = \"a.b\"\n")).is_none());
    }

    #[test]
    fn appdir_is_staged_for_velopack() {
        let p = project(&format!("{NOTES}\n"));
        let icon_dir = tempfile::tempdir().unwrap();
        let mut p = p;
        let png = icon_dir.path().join("icon.png");
        icon::test_png(&png, 512);
        p.config.app.icon = Some(png.display().to_string());
        let exe = icon_dir.path().join("demo");
        std::fs::write(&exe, b"\x7fELF").unwrap();
        let appdir = stage_appdir(&p, &exe, &p.nd_dir().join("package/linux")).unwrap();
        assert!(appdir.ends_with("Notes.AppDir"));
        for file in [
            "AppRun",
            "usr/bin/demo",
            "dev.formalsnake.notes.desktop",
            "dev.formalsnake.notes.png",
            "usr/share/applications/dev.formalsnake.notes.desktop",
            "usr/share/mime/packages/dev.formalsnake.notes.xml",
            "usr/share/icons/hicolor/256x256/apps/dev.formalsnake.notes.png",
        ] {
            assert!(appdir.join(file).exists(), "{file} missing");
        }
        let args = pack_args(&p, &Options::for_test("beta"), &appdir).join(" ");
        assert!(args.starts_with("[linux] pack --runtime linux-x64 --packId dev.formalsnake.notes"), "{args}");
        assert!(args.contains("--channel beta"));
    }
}
