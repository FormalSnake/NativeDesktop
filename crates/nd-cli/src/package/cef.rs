//! Chromium in a package: bundled only when the app's engine is CEF (`[webview] engine = "cef"`,
//! or `ND_WEBVIEW_ENGINE=cef` for one run), and proven absent otherwise.
//!
//! macOS layout, the one `nd-cef` looks for at run time:
//! `Contents/Frameworks/Chromium Embedded Framework.framework` and five helper apps,
//! `Contents/Frameworks/<App> Helper[ (GPU| Renderer| Plugin| Alerts)].app`, each running a
//! copy of `nd-cef-helper`.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::project::{Project, write, xml_escape};

pub const FRAMEWORK: &str = "Chromium Embedded Framework.framework";
pub const HELPER_BIN: &str = "nd-cef-helper";

/// The helper apps Chromium launches its processes from: name suffix and bundle id suffix.
pub const HELPERS: [(&str, &str); 5] = [
    ("", "helper"),
    (" (GPU)", "helper.gpu"),
    (" (Renderer)", "helper.renderer"),
    (" (Plugin)", "helper.plugin"),
    (" (Alerts)", "helper.alerts"),
];

/// Whether this package carries Chromium.
pub fn enabled(project: &Project) -> bool {
    match std::env::var("ND_WEBVIEW_ENGINE").ok().as_deref().map(str::trim) {
        Some("cef" | "chromium") => true,
        Some("system" | "webkit") => false,
        _ => project.config.uses_cef(),
    }
}

/// Hardened-runtime entitlements Chromium's renderer and GPU processes need (V8's JIT, and
/// loading the framework, which is not signed by the app's team).
pub const ENTITLEMENTS: [&str; 3] =
    ["cs.allow-jit", "cs.allow-unsigned-executable-memory", "cs.disable-library-validation"];

fn cargo() -> Command {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

/// The app's resolved dependency graph.
fn metadata(project: &Project) -> Result<serde_json::Value> {
    let output = cargo()
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(project.root.join("Cargo.toml"))
        .output()
        .context("running cargo metadata")?;
    if !output.status.success() {
        bail!("cargo metadata: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    serde_json::from_slice(&output.stdout).context("cargo metadata output")
}

fn package<'a>(meta: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
    meta["packages"].as_array()?.iter().find(|p| p["name"] == name)
}

/// The CEF binary distribution the app was built against: `ND_CEF_DIR`, else what the `cef`
/// crate's build script unpacked under `$CEF_PATH/<cef version>/`.
pub fn distribution(project: &Project) -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("ND_CEF_DIR").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let meta = metadata(project)?;
    let version = package(&meta, "cef")
        .and_then(|p| p["version"].as_str())
        .and_then(|v| v.split_once('+').map(|(_, cef)| cef.to_owned()))
        .context("the app does not depend on the cef crate: enable nd's `cef` feature")?;
    let root = std::env::var_os("CEF_PATH")
        .context("CEF_PATH is not set: point it at the directory the cef crate downloads into, or set ND_CEF_DIR")?;
    let arch = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" };
    let dir = PathBuf::from(root).join(&version).join(format!("cef_macos_{arch}"));
    if !dir.join(FRAMEWORK).exists() {
        bail!("no CEF {version} distribution at {} (build the app once so the cef crate fetches it)", dir.display());
    }
    Ok(dir)
}

/// Builds `nd-cef-helper` from the `nd-cef` the app depends on, into the app's target dir.
pub fn build_helper(project: &Project) -> Result<PathBuf> {
    let meta = metadata(project)?;
    let manifest = package(&meta, "nd-cef")
        .and_then(|p| p["manifest_path"].as_str())
        .context("the app does not depend on nd-cef: enable nd's `cef` feature")?;
    let status = cargo()
        .args(["build", "--release", "--bin", HELPER_BIN, "--manifest-path", manifest, "--target-dir"])
        .arg(&project.target_dir)
        .status()
        .context("building nd-cef-helper")?;
    if !status.success() {
        bail!("cargo build --bin {HELPER_BIN} failed ({status})");
    }
    let helper = project.target_dir.join("release").join(HELPER_BIN);
    if !helper.exists() {
        bail!("cargo build produced no {}", helper.display());
    }
    Ok(helper)
}

fn helper_plist(name: &str, id: &str, version: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n<dict>\n\
    <key>CFBundleExecutable</key>\n    <string>{name}</string>\n\
    <key>CFBundleIdentifier</key>\n    <string>{id}</string>\n\
    <key>CFBundleName</key>\n    <string>{name}</string>\n\
    <key>CFBundlePackageType</key>\n    <string>APPL</string>\n\
    <key>CFBundleShortVersionString</key>\n    <string>{version}</string>\n\
    <key>CFBundleVersion</key>\n    <string>{version}</string>\n\
    <key>LSUIElement</key>\n    <true/>\n\
    <key>LSMinimumSystemVersion</key>\n    <string>14.0</string>\n\
    <key>NSSupportsAutomaticGraphicsSwitching</key>\n    <true/>\n\
</dict>\n</plist>\n",
        name = xml_escape(name),
        id = xml_escape(id),
        version = xml_escape(version),
    )
}

/// Puts the framework and the helper apps into `app`'s `Contents/Frameworks`.
pub fn bundle_mac(project: &Project, app: &Path, dist: &Path, helper: &Path) -> Result<()> {
    let frameworks = app.join("Contents/Frameworks");
    std::fs::create_dir_all(&frameworks)?;
    // ditto keeps the framework's symlinks, which codesign requires.
    let status = Command::new("/usr/bin/ditto")
        .arg(dist.join(FRAMEWORK))
        .arg(frameworks.join(FRAMEWORK))
        .status()
        .context("copying the CEF framework")?;
    if !status.success() {
        bail!("ditto {FRAMEWORK} failed ({status})");
    }
    for (suffix, id_suffix) in HELPERS {
        let name = format!("{} Helper{suffix}", project.display_name());
        let contents = frameworks.join(format!("{name}.app")).join("Contents");
        std::fs::create_dir_all(contents.join("MacOS"))?;
        std::fs::copy(helper, contents.join("MacOS").join(&name))
            .with_context(|| format!("copying {} into {name}.app", helper.display()))?;
        write(
            &contents.join("Info.plist"),
            helper_plist(&name, &format!("{}.{id_suffix}", project.id()), &project.version),
        )?;
        write(&contents.join("PkgInfo"), "APPL????")?;
    }
    Ok(())
}

/// Byte strings only a Chromium build or the CEF glue would put in a file.
const MARKERS: [&[u8]; 4] = [b"Chromium Embedded Framework", b"cef_initialize", b"libcef", b"CefBrowserHost"];

/// Everything in `dir` that is Chromium: a file named like it, or a file containing one of
/// [`MARKERS`]. Empty for a package that does not use CEF.
pub fn chromium_findings(dir: &Path) -> Vec<String> {
    let mut findings = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        if path != dir && (name.contains("chromium") || name.contains("libcef") || name.starts_with("cef_")) {
            findings.push(format!("{}: named like Chromium", path.display()));
        }
        if meta.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&path) {
                stack.extend(entries.flatten().map(|e| e.path()));
            }
        } else if meta.is_file()
            && let Ok(bytes) = std::fs::read(&path)
            && let Some(marker) = MARKERS.iter().find(|m| bytes.windows(m.len()).any(|w| w == **m))
        {
            findings.push(format!("{}: contains {:?}", path.display(), String::from_utf8_lossy(marker)));
        }
    }
    findings.sort();
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn findings_name_chromium_files_and_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("A.app/Contents");
        std::fs::create_dir_all(app.join("MacOS")).unwrap();
        std::fs::write(app.join("MacOS/a"), b"plain binary bytes").unwrap();
        assert!(chromium_findings(dir.path()).is_empty());
        std::fs::write(app.join("MacOS/b"), b"xx cef_initialize yy").unwrap();
        std::fs::create_dir_all(app.join(format!("Frameworks/{FRAMEWORK}"))).unwrap();
        let found = chromium_findings(dir.path());
        assert_eq!(found.len(), 2, "{found:#?}");
        assert!(found.iter().any(|f| f.contains("cef_initialize")));
        assert!(found.iter().any(|f| f.contains("named like Chromium")));
    }

    #[test]
    fn helper_plists_lint() {
        let plist = helper_plist("Browser Helper (GPU)", "dev.formalsnake.browser.helper.gpu", "0.1.0");
        assert!(plist.contains("<string>Browser Helper (GPU)</string>"));
        assert!(plist.contains("<key>LSUIElement</key>\n    <true/>"));
        #[cfg(target_os = "macos")]
        {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("Info.plist");
            std::fs::write(&path, &plist).unwrap();
            let lint = Command::new("/usr/bin/plutil").arg("-lint").arg(&path).output().unwrap();
            assert!(lint.status.success(), "{}", String::from_utf8_lossy(&lint.stdout));
        }
    }
}
