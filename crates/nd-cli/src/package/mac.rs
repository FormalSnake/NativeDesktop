//! macOS: a `.app` from `nativedesktop.toml`, packed by Velopack (which adds its updater, signs
//! inside-out, notarizes and builds the `.pkg`), then a `.dmg` of the packed bundle.
//!
//! Signing is ad hoc (`-`) unless `[macos] signing_identity` is set. The Swift aggregate is
//! already statically linked into the binary by the app's `nd::build::app()`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use nd_config::FileRole;

use super::{Options, Packed, icon};
use crate::project::{Project, fresh_dir, write, xml_escape};
use crate::vpk;

const DEFAULT_MINIMUM: &str = "14.0";

pub fn package(project: &Project, opts: &Options) -> Result<Packed> {
    if !cfg!(target_os = "macos") {
        bail!("nd package mac runs on macOS: codesign, iconutil and hdiutil are Apple tools");
    }
    let work = project.nd_dir().join("package/mac");
    fresh_dir(&work)?;
    let exe = match &opts.binary {
        Some(path) => path.clone(),
        None => project.build_release(None)?,
    };
    let app = stage_bundle(project, &exe, &work.join("stage"))?;
    // vpk insists on the `.entitlements` extension.
    let entitlements = work.join("app.entitlements");
    write(&entitlements, entitlements_plist(&project.config.macos.entitlements))?;

    let vpk = vpk::locate()?;
    std::fs::create_dir_all(&opts.releases)?;
    vpk::run(&vpk, &pack_args(project, opts, &app, &entitlements))?;

    // The updatable bundle is the one inside the portable zip: Velopack added UpdateMac and its
    // manifest, then signed the whole bundle.
    let portable = opts.releases.join(format!("{}-{}-Portable.zip", project.id(), opts.channel));
    let out = work.join("out");
    std::fs::create_dir_all(&out)?;
    system("/usr/bin/ditto", |c| c.args(["-x", "-k"]).arg(&portable).arg(&out))?;
    let packed_app = out.join(format!("{}.app", project.display_name()));
    if !packed_app.exists() {
        bail!("vpk produced no {}", packed_app.display());
    }
    system("/usr/bin/codesign", |c| c.args(["--verify", "--deep", "--strict"]).arg(&packed_app))?;

    let dmg = out.join(format!("{}-{}.dmg", project.display_name(), project.version));
    make_dmg(project, &packed_app, &work.join("dmg"), &dmg)?;
    Ok(Packed { app: packed_app, installer: Some(dmg), releases: opts.releases.clone() })
}

/// The `vpk pack` arguments. Pure, so the mapping from `nativedesktop.toml` is unit tested.
pub fn pack_args(project: &Project, opts: &Options, app: &Path, entitlements: &Path) -> Vec<String> {
    let mac = &project.config.macos;
    let mut args: Vec<String> = vec![
        "pack".into(),
        "--packId".into(),
        project.id().into(),
        "--packVersion".into(),
        project.version.clone(),
        "--packDir".into(),
        app.display().to_string(),
        "--mainExe".into(),
        project.bin.clone(),
        "--packTitle".into(),
        project.display_name().into(),
        "--channel".into(),
        opts.channel.clone(),
        "--outputDir".into(),
        opts.releases.display().to_string(),
        "--signAppIdentity".into(),
        mac.signing_identity.clone().unwrap_or_else(|| "-".into()),
        "--signEntitlements".into(),
        entitlements.display().to_string(),
    ];
    if let Some(installer) = &mac.installer_identity {
        args.extend(["--signInstallIdentity".into(), installer.clone()]);
    }
    if mac.notarize
        && let Some(profile) = &mac.notary_profile
    {
        args.extend(["--notaryProfile".into(), profile.clone()]);
    }
    if let Some(notes) = &opts.notes {
        args.extend(["--releaseNotes".into(), notes.display().to_string()]);
    }
    args
}

fn stage_bundle(project: &Project, exe: &Path, stage: &Path) -> Result<PathBuf> {
    let app = stage.join(format!("{}.app", project.display_name()));
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    std::fs::create_dir_all(&macos)?;
    std::fs::create_dir_all(&resources)?;
    std::fs::copy(exe, macos.join(&project.bin)).with_context(|| format!("copying {}", exe.display()))?;
    let has_icon = match &project.config.app.icon {
        Some(source) => {
            let image = icon::load(&project.resolve(source))?;
            icon::write_icns(&image, stage, &resources.join("AppIcon.icns"))?;
            true
        }
        None => false,
    };
    write(&contents.join("Info.plist"), info_plist(project, has_icon))?;
    write(&contents.join("PkgInfo"), "APPL????")?;
    Ok(app)
}

fn key(out: &mut String, indent: usize, key: &str) {
    let _ = writeln!(out, "{:indent$}<key>{}</key>", "", xml_escape(key));
}

fn string(out: &mut String, indent: usize, name: &str, value: &str) {
    key(out, indent, name);
    let _ = writeln!(out, "{:indent$}<string>{}</string>", "", xml_escape(value));
}

fn strings(out: &mut String, indent: usize, name: &str, values: &[String]) {
    key(out, indent, name);
    let _ = writeln!(out, "{:indent$}<array>", "");
    for v in values {
        let _ = writeln!(out, "{:w$}<string>{}</string>", "", xml_escape(v), w = indent + 4);
    }
    let _ = writeln!(out, "{:indent$}</array>", "");
}

const PLIST_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n<dict>\n";

pub fn info_plist(project: &Project, has_icon: bool) -> String {
    let app = &project.config.app;
    let mut out = String::from(PLIST_HEAD);
    string(&mut out, 4, "CFBundleDevelopmentRegion", "en");
    string(&mut out, 4, "CFBundleExecutable", &project.bin);
    string(&mut out, 4, "CFBundleIdentifier", &app.id);
    string(&mut out, 4, "CFBundleInfoDictionaryVersion", "6.0");
    string(&mut out, 4, "CFBundleName", project.display_name());
    string(&mut out, 4, "CFBundleDisplayName", project.display_name());
    string(&mut out, 4, "CFBundlePackageType", "APPL");
    string(&mut out, 4, "CFBundleShortVersionString", &project.version);
    string(&mut out, 4, "CFBundleVersion", &project.version);
    string(
        &mut out,
        4,
        "LSMinimumSystemVersion",
        project.config.macos.minimum_system_version.as_deref().unwrap_or(DEFAULT_MINIMUM),
    );
    string(&mut out, 4, "NSPrincipalClass", "NSApplication");
    key(&mut out, 4, "NSHighResolutionCapable");
    out.push_str("    <true/>\n");
    if has_icon {
        string(&mut out, 4, "CFBundleIconFile", "AppIcon");
    }
    if !app.url_schemes.is_empty() {
        key(&mut out, 4, "CFBundleURLTypes");
        out.push_str("    <array>\n        <dict>\n");
        string(&mut out, 12, "CFBundleURLName", &app.id);
        strings(&mut out, 12, "CFBundleURLSchemes", &app.url_schemes);
        out.push_str("        </dict>\n    </array>\n");
    }
    if !app.file_associations.is_empty() {
        key(&mut out, 4, "CFBundleDocumentTypes");
        out.push_str("    <array>\n");
        for assoc in &app.file_associations {
            out.push_str("        <dict>\n");
            strings(&mut out, 12, "CFBundleTypeExtensions", &assoc.ext);
            string(&mut out, 12, "CFBundleTypeName", &assoc.type_name());
            let role = match assoc.role {
                FileRole::Editor => "Editor",
                FileRole::Viewer => "Viewer",
            };
            string(&mut out, 12, "CFBundleTypeRole", role);
            string(&mut out, 12, "LSHandlerRank", "Default");
            out.push_str("        </dict>\n");
        }
        out.push_str("    </array>\n");
    }
    out.push_str("</dict>\n</plist>\n");
    out
}

/// Hardened-runtime entitlements from `[macos] entitlements` short names
/// (`network.client` becomes `com.apple.security.network.client`). Passed to vpk explicitly so
/// it never falls back to its .NET defaults (JIT, unsigned executable memory).
pub fn entitlements_plist(names: &[String]) -> String {
    let mut out = String::from(PLIST_HEAD);
    for name in names {
        let full = if name.contains("com.apple.") { name.clone() } else { format!("com.apple.security.{name}") };
        key(&mut out, 4, &full);
        out.push_str("    <true/>\n");
    }
    out.push_str("</dict>\n</plist>\n");
    out
}

fn make_dmg(project: &Project, app: &Path, stage: &Path, dmg: &Path) -> Result<()> {
    fresh_dir(stage)?;
    let staged = stage.join(app.file_name().context("app name")?);
    system("/usr/bin/ditto", |c| c.arg(app).arg(&staged))?;
    #[cfg(unix)]
    std::os::unix::fs::symlink("/Applications", stage.join("Applications")).context("Applications link")?;
    system("/usr/bin/hdiutil", |c| {
        c.args(["create", "-quiet", "-ov", "-format", "UDZO", "-volname", project.display_name(), "-srcfolder"])
            .arg(stage)
            .arg(dmg)
    })?;
    let mac = &project.config.macos;
    if let Some(identity) = &mac.signing_identity {
        system("/usr/bin/codesign", |c| c.args(["--force", "--timestamp", "--sign", identity]).arg(dmg))?;
        if mac.notarize
            && let Some(profile) = &mac.notary_profile
        {
            system("/usr/bin/xcrun", |c| {
                c.args(["notarytool", "submit", "--keychain-profile", profile, "--wait"]).arg(dmg)
            })?;
            system("/usr/bin/xcrun", |c| c.args(["stapler", "staple"]).arg(dmg))?;
        }
    }
    Ok(())
}

/// Runs a system tool with Nix's SDK variables removed.
fn system(program: &str, configure: impl FnOnce(&mut Command) -> &mut Command) -> Result<()> {
    let mut command = Command::new(program);
    command.env_remove("SDKROOT").env_remove("DEVELOPER_DIR");
    configure(&mut command);
    let output = command.output().with_context(|| format!("running {program}"))?;
    if !output.status.success() {
        bail!("{program} failed ({}): {}", output.status, String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::tests::project;

    #[test]
    fn info_plist_carries_identity_documents_and_schemes() {
        let p = project(
            r#"
[app]
id = "dev.formalsnake.notes"
display_name = "Notes & Co"
url_schemes = ["notes"]
[[app.file_associations]]
ext = ["md", "markdown"]
[[app.file_associations]]
ext = ["txt"]
role = "viewer"
name = "Plain text"
[macos]
minimum_system_version = "15.0"
"#,
        );
        let plist = info_plist(&p, true);
        for needle in [
            "<key>CFBundleIdentifier</key>\n    <string>dev.formalsnake.notes</string>",
            "<string>Notes &amp; Co</string>",
            "<key>CFBundleExecutable</key>\n    <string>demo</string>",
            "<key>CFBundleShortVersionString</key>\n    <string>0.1.0</string>",
            "<key>LSMinimumSystemVersion</key>\n    <string>15.0</string>",
            "<string>notes</string>",
            "<string>markdown</string>",
            "<key>CFBundleTypeName</key>\n            <string>MD</string>",
            "<string>Viewer</string>",
            "<string>AppIcon</string>",
        ] {
            assert!(plist.contains(needle), "missing {needle:?} in\n{plist}");
        }
        #[cfg(target_os = "macos")]
        {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("Info.plist");
            std::fs::write(&path, &plist).unwrap();
            let lint = Command::new("/usr/bin/plutil").arg("-lint").arg(&path).output().unwrap();
            assert!(lint.status.success(), "{}", String::from_utf8_lossy(&lint.stdout));
        }
    }

    #[test]
    fn signing_defaults_to_ad_hoc_and_follows_the_config() {
        let opts = Options::for_test("stable");
        let ad_hoc = pack_args(&project("[app]\nid = \"a.b\"\n"), &opts, Path::new("A.app"), Path::new("e.plist"));
        let at = ad_hoc.iter().position(|a| a == "--signAppIdentity").unwrap();
        assert_eq!(ad_hoc[at + 1], "-");
        assert!(!ad_hoc.contains(&"--notaryProfile".to_string()));

        let signed = project(
            "[app]\nid = \"a.b\"\n[macos]\nsigning_identity = \"Developer ID Application: X\"\nnotarize = true\nnotary_profile = \"p\"\ninstaller_identity = \"Developer ID Installer: X\"\n",
        );
        let args = pack_args(&signed, &opts, Path::new("A.app"), Path::new("e.plist"));
        let joined = args.join(" ");
        assert!(joined.contains("--signAppIdentity Developer ID Application: X"));
        assert!(joined.contains("--notaryProfile p"));
        assert!(joined.contains("--signInstallIdentity Developer ID Installer: X"));
        assert!(joined.contains("--channel stable"));
    }

    #[test]
    fn entitlements_expand_short_names() {
        let plist = entitlements_plist(&["network.client".into(), "com.apple.security.cs.allow-jit".into()]);
        assert!(plist.contains("<key>com.apple.security.network.client</key>"));
        assert!(plist.contains("<key>com.apple.security.cs.allow-jit</key>"));
    }
}
