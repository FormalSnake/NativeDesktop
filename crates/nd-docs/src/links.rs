//! The link check: every link between pages lands on a page (and on a heading when it names
//! one), every link into the repository names a file or directory that exists, and every page
//! under `docs/content` is in the sidebar. External links are listed, not fetched.

use std::path::Path;

use crate::model::{Block, Inline, Link, Site};

#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub page: String,
    pub message: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "docs/content/{}.md: {}", self.page, self.message)
    }
}

pub fn visit_inlines<'a>(blocks: &'a [Block], f: &mut impl FnMut(&'a Inline)) {
    for block in blocks {
        match block {
            Block::Heading { text, .. } | Block::Paragraph(text) => text.iter().for_each(&mut *f),
            Block::List { items, .. } => items.iter().for_each(|i| visit_inlines(i, f)),
            Block::Quote(inner) => visit_inlines(inner, f),
            Block::Table { header, rows } => {
                header.iter().flatten().for_each(&mut *f);
                rows.iter().flatten().flatten().for_each(&mut *f);
            }
            Block::Code { .. } | Block::Rule | Block::Example(_) => {}
        }
    }
}

fn anchors(blocks: &[Block], out: &mut Vec<String>) {
    for block in blocks {
        match block {
            Block::Heading { anchor, .. } => out.push(anchor.clone()),
            Block::Quote(inner) => anchors(inner, out),
            Block::List { items, .. } => items.iter().for_each(|i| anchors(i, out)),
            _ => {}
        }
    }
}

/// `(findings, external links)`.
pub fn check(repo: &Path, site: &Site) -> (Vec<Finding>, Vec<String>) {
    let mut findings = Vec::new();
    let mut external = Vec::new();
    for page in &site.pages {
        let mut links = Vec::new();
        visit_inlines(&page.blocks, &mut |run| {
            if let Some(link) = &run.link
                && !links.contains(link)
            {
                links.push(link.clone());
            }
        });
        for link in links {
            match link {
                Link::Page { id, anchor } => match site.page(&id) {
                    None => findings.push(Finding { page: page.id.clone(), message: format!("no page `{id}`") }),
                    Some(target) => {
                        if let Some(anchor) = anchor {
                            let mut known = Vec::new();
                            anchors(&target.blocks, &mut known);
                            if !known.contains(&anchor) {
                                findings.push(Finding {
                                    page: page.id.clone(),
                                    message: format!("`{id}` has no heading `#{anchor}`"),
                                });
                            }
                        }
                    }
                },
                Link::Repo(path) => {
                    if !repo.join(&path).exists() {
                        findings.push(Finding { page: page.id.clone(), message: format!("no file `{path}`") });
                    }
                }
                Link::External(url) => {
                    if !external.contains(&url) {
                        external.push(url);
                    }
                }
            }
        }
    }
    for page in &site.pages {
        if !site.sections.iter().any(|s| s.pages.contains(&page.id)) {
            findings.push(Finding { page: page.id.clone(), message: "not in docs/content/nav.toml".into() });
        }
    }
    (findings, external)
}
