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
    ("src/main.rs", include_str!("../templates/src/main.rs.tmpl")),
    ("src/lib.rs", include_str!("../templates/src/lib.rs.tmpl")),
    ("src/app.rs", include_str!("../templates/src/app.rs.tmpl")),
    ("src/demo.rs", include_str!("../templates/src/demo.rs.tmpl")),
    ("tests/app.rs", include_str!("../templates/tests/app.rs.tmpl")),
    ("tests/appearance.rs", include_str!("../templates/tests/appearance.rs.tmpl")),
    ("CLAUDE.md", include_str!("../templates/CLAUDE.md.tmpl")),
    ("README.md", include_str!("../templates/README.md.tmpl")),
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

/// The `[workspace]` table of a standalone project. Outside nd's own workspace a path
/// dependency on nd still needs nd's GPUI patch queue, which only the root manifest can name.
fn workspace(standalone: bool, nd_path: Option<&Path>) -> Result<String> {
    if !standalone {
        return Ok(String::new());
    }
    let mut out = "\n[workspace]\n".to_string();
    if let Some(nd) = nd_path {
        let nd = std::fs::canonicalize(nd).with_context(|| format!("--nd-path {}", nd.display()))?;
        let vendor = nd.parent().unwrap_or(&nd).join("nd-gpui/vendor");
        out.push_str("\n# nd's GPUI patch queue; cargo resolves the unpatched crates.io releases without it.\n[patch.crates-io]\n");
        for krate in ["gpui-pre", "gpui-fast", "gpui-pre-linux", "gpui-fast-linux", "gpui-pre-wgpu", "gpui-fast-wgpu"] {
            out.push_str(&format!("{krate} = {{ path = \"{}\" }}\n", vendor.join(krate).display()));
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
                let path = path.display();
                (
                    format!("{{ path = \"{path}\" }}"),
                    format!("{{ path = \"{path}\", features = [\"test\", \"audit\"] }}"),
                    format!("{{ path = \"{path}\", default-features = false, features = [\"build\"] }}"),
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
                ("{{workspace}}", workspace(standalone, nd_path)?),
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
    fn a_standalone_path_project_names_the_gpui_patches() {
        let nd = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../nd");
        let table = super::workspace(true, Some(&nd)).unwrap();
        assert!(table.contains("[patch.crates-io]") && table.contains("nd-gpui/vendor/gpui-pre-wgpu"), "{table}");
        assert_eq!(super::workspace(false, Some(&nd)).unwrap(), "");
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
}
