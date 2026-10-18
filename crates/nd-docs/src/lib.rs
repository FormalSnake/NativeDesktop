//! The docs content model shared by the docs app (`docs/app`) and `nd docs`: pages from
//! `docs/content`, code cut from `examples/` by anchor, the example registry
//! (`docs/examples.toml`), per-OS captures, the generated component reference, the link and
//! prose checks, and the static export: HTML pages, raw markdown and `llms.txt`.
//!
//! No UI here: `nd-cli` depends on it, and the CLI stays free of the framework's runtime.

pub mod captures;
pub mod components;
pub mod examples;
pub mod html;
pub mod links;
pub mod markdown;
pub mod model;
pub mod prose;
pub mod snippets;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

pub use model::{Block, Inline, Link, Page, Section, Site, SourceRef};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Nav {
    section: Vec<Section>,
}

/// The repository root above `start`: the first ancestor with `docs/content/nav.toml`.
pub fn find_repo(start: &Path) -> Option<PathBuf> {
    start.ancestors().find(|d| d.join("docs/content/nav.toml").is_file()).map(Path::to_path_buf)
}

/// The workspace version, which is also the framework version captures are keyed by.
pub fn framework_version(repo: &Path) -> Result<String> {
    let text = std::fs::read_to_string(repo.join("Cargo.toml"))?;
    let manifest: toml::Table = toml::from_str(&text)?;
    manifest
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .context("Cargo.toml has no [workspace.package] version")
}

fn md_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            md_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
    Ok(())
}

/// Loads and resolves every page. Fails on the first page that does not parse or names a
/// snippet, example or directive that does not exist.
pub fn load(repo: &Path) -> Result<Site> {
    let content = repo.join("docs/content");
    let nav: Nav =
        toml::from_str(&std::fs::read_to_string(content.join("nav.toml"))?).context("docs/content/nav.toml")?;
    let examples = examples::load(repo, &framework_version(repo)?)?;
    let components = || components::reference(repo);
    let cx = markdown::PageContext { repo, examples: &examples, components: &components };
    let mut files = Vec::new();
    md_files(&content, &mut files)?;
    files.sort();
    let mut pages = Vec::new();
    for file in files {
        let id = file.strip_prefix(&content)?.with_extension("").to_string_lossy().replace('\\', "/");
        let text = std::fs::read_to_string(&file)?;
        pages.push(markdown::page(&id, &text, &cx)?);
    }
    for section in &nav.section {
        for id in &section.pages {
            if !pages.iter().any(|p| &p.id == id) {
                bail!("docs/content/nav.toml: `{id}` has no docs/content/{id}.md");
            }
        }
    }
    // Sidebar order.
    let order = |id: &str| nav.section.iter().flat_map(|s| &s.pages).position(|p| p == id).unwrap_or(usize::MAX);
    pages.sort_by_key(|p| order(&p.id));
    Ok(Site { sections: nav.section, pages, examples })
}

/// `<out>/<id>.md` and `<out>/<id>.html` for every page, `<out>/llms.txt` (an index) and
/// `llms-full.txt` (every page in sidebar order), for agents and anyone reading without the app.
/// Links in the markdown are rewritten to absolute URLs, so a page read on its own or inside
/// `llms-full.txt` still points somewhere real.
pub fn export(site: &Site, captures: &captures::Manifest, out: &Path) -> Result<()> {
    std::fs::create_dir_all(out)?;
    let site_url = html::SITE_URL;
    let mut index = format!(
        "# NativeDesktop\n\n> One Rust view tree, native on macOS (AppKit), Linux and Windows (GPUI), and plain GPUI on the web.\n\n\
         Every page below is markdown. All of them in one file: {site_url}/llms-full.txt. \
         Building an app with nd: start at {site_url}/agents/index.md.\n\n",
    );
    let mut full = String::new();
    for section in &site.sections {
        index.push_str(&format!("## {}\n\n", section.title));
        for id in &section.pages {
            let Some(page) = site.page(id) else { continue };
            let path = out.join(format!("{id}.md"));
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let markdown = absolute_links(&page.markdown, id);
            std::fs::write(&path, &markdown)?;
            let description =
                if page.description.is_empty() { String::new() } else { format!(": {}", page.description) };
            index.push_str(&format!("- [{}]({site_url}/{id}.md){description}\n", page.title));
            full.push_str(&format!("<!-- {site_url}/{id}.md -->\n\n"));
            full.push_str(&markdown);
            full.push_str("\n\n");
        }
        index.push('\n');
    }
    std::fs::write(out.join("llms.txt"), index)?;
    std::fs::write(out.join("llms-full.txt"), full)?;
    html::write(site, captures, out)
}

/// `markdown` from page `id` with every relative inline link made absolute: pages to their
/// `.md` on the site, repository files to GitHub. Same-page anchors stay as they are.
pub fn absolute_links(markdown: &str, id: &str) -> String {
    use pulldown_cmark::{Event, LinkType, Options, Parser, Tag};
    let base = match id.rsplit_once('/') {
        Some((dir, _)) => format!("docs/content/{dir}"),
        None => "docs/content".to_owned(),
    };
    let mut edits = Vec::new();
    for (event, range) in Parser::new_ext(markdown, Options::ENABLE_TABLES).into_offset_iter() {
        let Event::Start(Tag::Link { link_type: LinkType::Inline, dest_url, .. }) = event else { continue };
        if dest_url.starts_with('#') {
            continue;
        }
        let href = match markdown::classify(&dest_url, id, &base) {
            Link::Page { id, anchor } => {
                format!("{}/{id}.md{}", html::SITE_URL, anchor.map(|a| format!("#{a}")).unwrap_or_default())
            }
            Link::Repo(path) => html::repo_url(&path),
            Link::External(_) => continue,
        };
        let text = &markdown[range.clone()];
        let Some(open) = text.rfind("](") else { continue };
        let start = range.start + open + 2;
        if markdown[start..].starts_with(dest_url.as_ref()) {
            edits.push((start..start + dest_url.len(), href));
        }
    }
    let mut out = markdown.to_owned();
    for (range, href) in edits.into_iter().rev() {
        out.replace_range(range, &href);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_real_site_loads() {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let site = super::load(&repo).unwrap();
        assert!(!site.pages.is_empty());
        let (findings, _) = super::links::check(&repo, &site);
        assert!(findings.is_empty(), "{findings:#?}");
    }

    #[test]
    fn links_become_absolute() {
        let md = "See [theming](../concepts/theming.md#tokens), [here](#x), [src](../../../examples/hello) \
                  and [web](https://example.com).";
        let out = super::absolute_links(md, "guides/web");
        assert!(out.contains("(https://native.formalsnake.dev/concepts/theming.md#tokens)"), "{out}");
        assert!(out.contains("[here](#x)"));
        assert!(out.contains("(https://github.com/FormalSnake/NativeDesktop/blob/main/examples/hello)"));
        assert!(out.contains("(https://example.com)"));
    }
}
