//! `nd new <name>`: the project tree of ARCHITECTURE.md 15.2, as far as the framework ships it.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Args;

#[derive(Args, Debug)]
pub struct NewArgs {
    /// Directory to create; its last component is the package name.
    pub name: PathBuf,
    /// Reverse-DNS app id. Defaults to `com.example.<name>`.
    #[arg(long)]
    pub id: Option<String>,
    /// Depend on a local checkout of the `nd` crate instead of crates.io.
    #[arg(long, env = "ND_PATH")]
    pub nd_path: Option<PathBuf>,
    /// Add an empty `[workspace]` so the app builds on its own even inside another workspace.
    #[arg(long)]
    pub standalone: bool,
}

/// (path in the new project, template).
const FILES: &[(&str, &str)] = &[
    ("Cargo.toml", include_str!("../templates/Cargo.toml.tmpl")),
    ("nativedesktop.toml", include_str!("../templates/nativedesktop.toml.tmpl")),
    ("build.rs", include_str!("../templates/build.rs.tmpl")),
    ("rustfmt.toml", include_str!("../templates/rustfmt.toml.tmpl")),
    ("clippy.toml", include_str!("../templates/clippy.toml.tmpl")),
    (".gitignore", include_str!("../templates/gitignore.tmpl")),
    ("justfile", include_str!("../templates/justfile.tmpl")),
    (".config/nextest.toml", include_str!("../templates/.config/nextest.toml.tmpl")),
    (".cargo/config.toml", include_str!("../templates/.cargo/config.toml.tmpl")),
    ("src/main.rs", include_str!("../templates/src/main.rs.tmpl")),
    ("src/lib.rs", include_str!("../templates/src/lib.rs.tmpl")),
    ("src/app.rs", include_str!("../templates/src/app.rs.tmpl")),
    ("src/demo.rs", include_str!("../templates/src/demo.rs.tmpl")),
    ("tests/app.rs", include_str!("../templates/tests/app.rs.tmpl")),
    ("tests/appearance.rs", include_str!("../templates/tests/appearance.rs.tmpl")),
    ("CLAUDE.md", include_str!("../templates/CLAUDE.md.tmpl")),
    ("README.md", include_str!("../templates/README.md.tmpl")),
    ("docs/agents/patterns.md", include_str!("../templates/docs/agents/patterns.md")),
    ("docs/agents/widgets.md", include_str!("../templates/docs/agents/widgets.md")),
    ("docs/agents/testing.md", include_str!("../templates/docs/agents/testing.md")),
    ("docs/agents/styling.md", include_str!("../templates/docs/agents/styling.md")),
    ("docs/agents/performance.md", include_str!("../templates/docs/agents/performance.md")),
    ("docs/agents/donts.md", include_str!("../templates/docs/agents/donts.md")),
];

const RESERVED: &[&str] = &["nd", "test", "std", "core", "alloc", "proc_macro", "self", "crate", "super"];

pub fn run(args: NewArgs) -> Result<()> {
    let name = package_name(&args.name)?;
    let root = &args.name;
    if root.exists() && root.read_dir().with_context(|| format!("reading {}", root.display()))?.next().is_some() {
        bail!("{} exists and is not empty", root.display());
    }
    let vars = Vars::new(&name, args.id.as_deref(), args.nd_path.as_deref(), args.standalone)?;
    for (path, template) in FILES {
        let target = root.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&target, vars.render(template)).with_context(|| format!("writing {}", target.display()))?;
    }
    if let Some(lock) = args.nd_path.as_deref().and_then(workspace_lock) {
        // A fresh resolve takes the newest releases, some yanked or broken since nd pinned them;
        // starting from nd's lockfile resolves the app to the versions nd is built and tested with.
        std::fs::copy(&lock, root.join("Cargo.lock")).with_context(|| format!("copying {}", lock.display()))?;
    }
    println!("created {} at {}", name, root.display());
    println!("next: cd {} && nd check && nd test", root.display());
    Ok(())
}

pub fn package_name(path: &Path) -> Result<String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .context("the project name must be a plain directory name")?
        .to_string();
    let mut chars = name.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if !valid {
        bail!("`{name}` is not a valid name: start with a lowercase letter, then lowercase letters, digits, - or _");
    }
    if RESERVED.contains(&name.replace('-', "_").as_str()) {
        bail!("`{name}` is reserved; pick another name");
    }
    Ok(name)
}

struct Vars {
    pairs: Vec<(&'static str, String)>,
}

/// The lockfile of the workspace the `nd` crate at `nd_path` belongs to.
fn workspace_lock(nd_path: &Path) -> Option<PathBuf> {
    let nd = std::fs::canonicalize(nd_path).ok()?;
    nd.ancestors().skip(1).map(|dir| dir.join("Cargo.lock")).find(|lock| lock.is_file())
}

/// The root tables: `[workspace]` for a standalone project, and nd's GPUI patch queue for a path
/// dependency on nd, which cargo only reads from the root manifest and which an app outside nd's
/// own workspace needs, since cargo resolves the unpatched crates.io releases without it.
fn root_tables(standalone: bool, nd_path: Option<&Path>) -> Result<String> {
    let mut out = String::new();
    if standalone {
        out.push_str("\n[workspace]\n");
    }
    if let Some(nd) = nd_path {
        let nd = std::fs::canonicalize(nd).with_context(|| format!("--nd-path {}", nd.display()))?;
        let vendor = nd.parent().unwrap_or(&nd).join("nd-gpui/vendor");
        out.push_str("\n# nd's GPUI patch queue.\n[patch.crates-io]\n");
        for krate in ["gpui-pre", "gpui-fast", "gpui-pre-linux", "gpui-fast-linux", "gpui-pre-wgpu", "gpui-fast-wgpu"] {
            let path = vendor.join(krate).display().to_string();
            let path = path.strip_prefix(r"\\?\").unwrap_or(&path).to_string();
            out.push_str(&format!("{krate} = {{ path = '{path}' }}\n"));
        }
    }
    Ok(out)
}

impl Vars {
    fn new(name: &str, id: Option<&str>, nd_path: Option<&Path>, standalone: bool) -> Result<Self> {
        let krate = name.replace('-', "_");
        let title = name
            .split(['-', '_'])
            .filter(|w| !w.is_empty())
            .map(|w| {
                let mut c = w.chars();
                c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" ");
        let (nd_dep, nd_dev_dep, nd_build_dep) = match nd_path {
            Some(path) => {
                let path = std::fs::canonicalize(path).with_context(|| format!("--nd-path {}", path.display()))?;
                // A TOML literal string, so Windows backslashes stay as written; canonicalize's
                // verbatim `\\?\` prefix goes, since build scripts mishandle it.
                let path = path.display().to_string();
                let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
                (
                    format!("{{ path = '{path}' }}"),
                    format!("{{ path = '{path}', features = [\"test\", \"audit\"] }}"),
                    format!("{{ path = '{path}', default-features = false, features = [\"build\"] }}"),
                )
            }
            None => (
                "\"0.1\"".to_string(),
                "{ version = \"0.1\", features = [\"test\", \"audit\"] }".to_string(),
                "{ version = \"0.1\", default-features = false, features = [\"build\"] }".to_string(),
            ),
        };
        Ok(Self {
            pairs: vec![
                ("{{name}}", name.to_string()),
                ("{{crate}}", krate.clone()),
                ("{{title}}", title),
                ("{{id}}", id.map_or_else(|| format!("com.example.{krate}"), str::to_string)),
                ("{{env}}", krate.to_ascii_uppercase()),
                ("{{nd_dep}}", nd_dep),
                ("{{nd_dev_dep}}", nd_dev_dep),
                ("{{nd_build_dep}}", nd_build_dep),
                ("{{workspace}}", root_tables(standalone, nd_path)?),
            ],
        })
    }

    fn render(&self, template: &str) -> String {
        self.pairs.iter().fold(template.to_string(), |text, (key, value)| text.replace(key, value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_validated() {
        assert!(package_name(Path::new("demo")).is_ok());
        assert!(package_name(Path::new("some/path/my-app_2")).is_ok());
        assert!(package_name(Path::new("Demo")).is_err());
        assert!(package_name(Path::new("2fast")).is_err());
        assert!(package_name(Path::new("nd")).is_err());
        assert!(package_name(Path::new("my app")).is_err());
    }

    #[test]
    fn a_path_project_names_the_gpui_patches_standalone_or_not() {
        let nd = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../nd");
        for standalone in [true, false] {
            let table = root_tables(standalone, Some(&nd)).unwrap();
            assert!(table.contains("[patch.crates-io]") && table.contains("nd-gpui/vendor/gpui-pre-wgpu"), "{table}");
            assert_eq!(table.contains("[workspace]"), standalone, "{table}");
        }
        assert_eq!(root_tables(false, None).unwrap(), "");
    }

    #[test]
    fn macos_links_through_apples_clang() {
        let (_, config) = FILES.iter().find(|(p, _)| *p == ".cargo/config.toml").expect("a cargo config is written");
        for target in ["aarch64-apple-darwin", "x86_64-apple-darwin"] {
            let table = format!("[target.{target}]\nlinker = \"/usr/bin/clang\"\n");
            assert!(config.contains(&table), "{target} in\n{config}");
        }
    }

    #[test]
    fn nd_path_generates_a_cargo_patch_block_pointing_at_the_vendor_dir() {
        let nd = std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../nd")).unwrap();
        let vendor = nd.parent().unwrap().join("nd-gpui/vendor");
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("my-app");
        run(NewArgs { name: root.clone(), id: None, nd_path: Some(nd), standalone: true }).unwrap();
        let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        assert!(manifest.contains("\n[workspace]\n"), "{manifest}");
        let lock = std::fs::read_to_string(root.join("Cargo.lock")).expect("nd's lockfile is copied");
        assert!(lock.contains("name = \"nd-ui\""), "the lockfile is nd's workspace's");
        let (_, patch) = manifest.split_once("[patch.crates-io]\n").expect("[patch.crates-io] is present");
        assert_eq!(patch.lines().filter(|l| l.contains("path = ")).count(), 6, "{patch}");
        for krate in ["gpui-pre", "gpui-fast", "gpui-pre-linux", "gpui-fast-linux", "gpui-pre-wgpu", "gpui-fast-wgpu"] {
            let line = format!("{krate} = {{ path = '{}' }}", vendor.join(krate).display());
            assert!(patch.contains(&line), "{line} missing from\n{patch}");
            assert!(vendor.join(krate).join("Cargo.toml").exists(), "{krate} exists in the vendor dir");
        }
    }

    #[test]
    fn every_placeholder_is_filled() {
        let vars = Vars::new("my-app", None, None, true).unwrap();
        for (path, template) in FILES {
            let out = vars.render(template);
            assert!(!out.contains("{{"), "{path} has an unfilled placeholder");
            assert!(!out.contains('\u{2014}'), "{path} has an em dash");
        }
        assert!(vars.render("{{title}} {{env}} {{id}}").contains("My App MY_APP com.example.my_app"));
    }

    #[test]
    fn agent_docs_are_written() {
        let vars = Vars::new("my-app", None, None, true).unwrap();
        let wanted = ["CLAUDE.md", "patterns", "widgets", "testing", "styling", "performance", "donts"];
        for name in wanted {
            let path = if name == "CLAUDE.md" { name.to_string() } else { format!("docs/agents/{name}.md") };
            let (_, template) =
                FILES.iter().find(|(p, _)| *p == path).unwrap_or_else(|| panic!("{path} is not written"));
            let out = vars.render(template);
            assert!(out.starts_with("# "), "{path} starts with its title");
            assert!(!out.contains('\u{2014}') && !out.contains(" \u{2013} "), "{path} has an em or en dash");
            if path != "CLAUDE.md" {
                assert!(!template.contains("{{"), "{path} is copied as is and takes no placeholders");
            }
        }
        let claude = vars.render(FILES.iter().find(|(p, _)| *p == "CLAUDE.md").unwrap().1);
        for page in wanted.iter().skip(1) {
            assert!(claude.contains(&format!("docs/agents/{page}.md")), "CLAUDE.md links docs/agents/{page}.md");
        }
    }
}
