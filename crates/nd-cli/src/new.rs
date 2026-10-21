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
    /// Depend on a local checkout of the `nd` crate instead of this CLI's release tag.
    #[arg(long, env = "ND_PATH")]
    pub nd_path: Option<PathBuf>,
    /// The git repository nd and its GPUI fork come from, for a mirror or a fork.
    #[arg(long, env = "ND_GIT", default_value = REPO)]
    pub nd_git: String,
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

/// Where releases are tagged. nd depends on a patched GPUI, so it is not on crates.io.
pub const REPO: &str = "https://github.com/FormalStudio/NativeDesktop";

/// nd's workspace manifest and lockfile at the commit this CLI was built from.
const WORKSPACE_MANIFEST: &str = include_str!("../../../Cargo.toml");
const WORKSPACE_LOCK: &str = include_str!("../../../Cargo.lock");

/// The release tag of this CLI, which `nd new` depends on.
pub fn release_tag() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

/// The packages nd's workspace patches with its vendored GPUI fork, read off the block
/// `scripts/gpui-fork vendor` writes.
pub fn gpui_patch_crates() -> Vec<&'static str> {
    let block = WORKSPACE_MANIFEST.split("# gpui-fork: begin").nth(1).unwrap_or_default();
    let block = block.split("# gpui-fork: end").next().unwrap_or_default();
    block
        .lines()
        .filter(|line| line.contains("path = \"crates/nd-gpui/vendor/"))
        .filter_map(|line| line.split_once(" = ").map(|(name, _)| name.trim()))
        .collect()
}

/// Where the app gets nd from.
enum Source<'a> {
    Path(String),
    Git { url: &'a str, tag: String },
}

impl Source<'_> {
    /// The inside of a dependency table naming this source.
    fn table(&self) -> String {
        match self {
            // A TOML literal string, so Windows backslashes stay as written.
            Source::Path(path) => format!("path = '{path}'"),
            Source::Git { url, tag } => format!("git = \"{url}\", tag = \"{tag}\""),
        }
    }
}

const RESERVED: &[&str] = &["nd", "test", "std", "core", "alloc", "proc_macro", "self", "crate", "super"];

pub fn run(args: NewArgs) -> Result<()> {
    let name = package_name(&args.name)?;
    let root = &args.name;
    if root.exists() && root.read_dir().with_context(|| format!("reading {}", root.display()))?.next().is_some() {
        bail!("{} exists and is not empty", root.display());
    }
    let source = match args.nd_path.as_deref() {
        Some(path) => Source::Path(plain_path(path)?),
        None => Source::Git { url: &args.nd_git, tag: release_tag() },
    };
    let vars = Vars::new(&name, args.id.as_deref(), &source, args.standalone);
    for (path, template) in FILES {
        let target = root.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&target, vars.render(template)).with_context(|| format!("writing {}", target.display()))?;
    }
    // A fresh resolve takes the newest releases, some yanked or broken since nd pinned them;
    // starting from nd's lockfile resolves the app to the versions nd is built and tested with.
    let lock_path = root.join("Cargo.lock");
    match args.nd_path.as_deref().and_then(workspace_lock) {
        Some(lock) => std::fs::copy(&lock, &lock_path).map(drop),
        None => std::fs::write(&lock_path, WORKSPACE_LOCK),
    }
    .with_context(|| format!("writing {}", lock_path.display()))?;
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

/// `path` canonicalized, without the verbatim `\\?\` prefix canonicalize gives on Windows, which
/// build scripts mishandle.
fn plain_path(path: &Path) -> Result<String> {
    let path = std::fs::canonicalize(path).with_context(|| format!("--nd-path {}", path.display()))?;
    let path = path.display().to_string();
    Ok(path.strip_prefix(r"\\?\").unwrap_or(&path).to_string())
}

/// The root tables: `[workspace]` for a standalone project, and nd's GPUI patch queue, which
/// cargo only reads from the root manifest and which an app outside nd's own workspace needs,
/// since cargo resolves the unpatched crates.io releases without it.
fn root_tables(standalone: bool, source: &Source) -> String {
    let mut out = String::new();
    if standalone {
        out.push_str("\n[workspace]\n");
    }
    out.push_str("\n# nd's GPUI patch queue.\n[patch.crates-io]\n");
    for krate in gpui_patch_crates() {
        let table = match source {
            Source::Path(nd) => {
                let vendor = Path::new(nd).parent().unwrap_or(Path::new(nd)).join("nd-gpui").join("vendor").join(krate);
                Source::Path(vendor.display().to_string()).table()
            }
            Source::Git { .. } => source.table(),
        };
        out.push_str(&format!("{krate} = {{ {table} }}\n"));
    }
    out
}

impl Vars {
    fn new(name: &str, id: Option<&str>, source: &Source, standalone: bool) -> Self {
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
        let nd = source.table();
        Self {
            pairs: vec![
                ("{{name}}", name.to_string()),
                ("{{crate}}", krate.clone()),
                ("{{title}}", title),
                ("{{id}}", id.map_or_else(|| format!("com.example.{krate}"), str::to_string)),
                ("{{env}}", krate.to_ascii_uppercase()),
                ("{{nd_dep}}", format!("{{ {nd} }}")),
                ("{{nd_dev_dep}}", format!("{{ {nd}, features = [\"test\", \"audit\"] }}")),
                ("{{nd_build_dep}}", format!("{{ {nd}, default-features = false, features = [\"build\"] }}")),
                ("{{workspace}}", root_tables(standalone, source)),
            ],
        }
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
    fn the_patch_block_names_every_vendored_gpui_crate_standalone_or_not() {
        let crates = gpui_patch_crates();
        for krate in ["gpui-pre", "gpui-fast", "gpui-pre-linux", "gpui-pre-wgpu", "gpui-pre-windows", "gpui-pre-apple"]
        {
            assert!(crates.contains(&krate), "{krate} in {crates:?}");
        }
        let nd = plain_path(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../nd")).unwrap();
        let git = Source::Git { url: REPO, tag: "v1.2.3".into() };
        for standalone in [true, false] {
            let table = root_tables(standalone, &Source::Path(nd.clone()));
            let wgpu = Path::new(&nd).parent().unwrap().join("nd-gpui").join("vendor").join("gpui-pre-wgpu");
            assert!(table.contains("[patch.crates-io]") && table.contains(&*wgpu.display().to_string()), "{table}");
            assert_eq!(table.contains("[workspace]"), standalone, "{table}");
            let table = root_tables(standalone, &git);
            let line = format!("gpui-pre-windows = {{ git = \"{REPO}\", tag = \"v1.2.3\" }}\n");
            assert!(table.contains(&line), "{table}");
            assert_eq!(table.matches("git = ").count(), crates.len(), "{table}");
        }
    }

    #[test]
    fn a_release_project_depends_on_this_clis_tag_and_carries_nds_lockfile() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("my-app");
        run(NewArgs { name: root.clone(), id: None, nd_path: None, nd_git: REPO.into(), standalone: false }).unwrap();
        let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        let tag = format!("v{}", env!("CARGO_PKG_VERSION"));
        assert!(manifest.contains(&format!("nd = {{ git = \"{REPO}\", tag = \"{tag}\" }}\n")), "{manifest}");
        assert!(!manifest.contains("path = "), "{manifest}");
        let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
        assert!(lock.contains("name = \"nd-ui\""), "the lockfile is nd's workspace's");
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
        let vendor = PathBuf::from(plain_path(&nd).unwrap()).parent().unwrap().join("nd-gpui").join("vendor");
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("my-app");
        run(NewArgs { name: root.clone(), id: None, nd_path: Some(nd), nd_git: REPO.into(), standalone: true })
            .unwrap();
        let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        assert!(manifest.contains("\n[workspace]\n"), "{manifest}");
        let lock = std::fs::read_to_string(root.join("Cargo.lock")).expect("nd's lockfile is copied");
        assert!(lock.contains("name = \"nd-ui\""), "the lockfile is nd's workspace's");
        let (_, patch) = manifest.split_once("[patch.crates-io]\n").expect("[patch.crates-io] is present");
        assert_eq!(patch.lines().filter(|l| l.contains("path = ")).count(), gpui_patch_crates().len(), "{patch}");
        for krate in gpui_patch_crates() {
            let line = format!("{krate} = {{ path = '{}' }}", vendor.join(krate).display());
            assert!(patch.contains(&line), "{line} missing from\n{patch}");
            assert!(vendor.join(krate).join("Cargo.toml").exists(), "{krate} exists in the vendor dir");
        }
    }

    #[test]
    fn every_placeholder_is_filled() {
        let vars = Vars::new("my-app", None, &Source::Git { url: REPO, tag: release_tag() }, true);
        for (path, template) in FILES {
            let out = vars.render(template);
            assert!(!out.contains("{{"), "{path} has an unfilled placeholder");
            assert!(!out.contains('\u{2014}'), "{path} has an em dash");
        }
        assert!(vars.render("{{title}} {{env}} {{id}}").contains("My App MY_APP com.example.my_app"));
    }

    #[test]
    fn agent_docs_are_written() {
        let vars = Vars::new("my-app", None, &Source::Git { url: REPO, tag: release_tag() }, true);
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
