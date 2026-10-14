//! What a page is once its markdown is parsed and its directives resolved. The docs app renders
//! these blocks; the exporter writes the resolved markdown next to them.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Site {
    pub sections: Vec<Section>,
    pub pages: Vec<Page>,
    pub examples: Vec<crate::examples::Example>,
}

impl Site {
    pub fn page(&self, id: &str) -> Option<&Page> {
        self.pages.iter().find(|p| p.id == id)
    }

    pub fn example(&self, name: &str) -> Option<&crate::examples::Example> {
        self.examples.iter().find(|e| e.name == name)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub title: String,
    /// Page ids in sidebar order.
    pub pages: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Page {
    /// The path under `docs/content` without `.md`: `concepts/theming`.
    pub id: String,
    pub title: String,
    pub description: String,
    pub blocks: Vec<Block>,
    /// The page's markdown with snippets and generated parts filled in, for `<id>.md` and
    /// `llms.txt`.
    pub markdown: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Block {
    Heading {
        level: u8,
        anchor: String,
        text: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    Code {
        lang: String,
        code: String,
        source: Option<SourceRef>,
    },
    List {
        ordered: bool,
        items: Vec<Vec<Block>>,
    },
    Quote(Vec<Block>),
    Table {
        header: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    Rule,
    /// The four-tab embed of an example app.
    Example(String),
}

/// A run of text with one style. Paragraphs are lists of runs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Inline {
    pub text: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub code: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub strong: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub emph: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<Link>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Link {
    /// Another docs page, with an optional heading anchor.
    Page {
        id: String,
        anchor: Option<String>,
    },
    /// A file in the repository, relative to its root.
    Repo(String),
    External(String),
}

/// Where a code block was cut from: `examples/hello/src/lib.rs`, anchor `counter`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceRef {
    pub path: String,
    pub anchor: Option<String>,
    /// 1-based line of the first line shown.
    pub line: usize,
}

pub fn plain(runs: &[Inline]) -> String {
    runs.iter().map(|r| r.text.as_str()).collect()
}

/// GitHub's heading anchor: lowercase, spaces to `-`, punctuation other than `-` and `_` dropped.
pub fn slug(text: &str) -> String {
    text.trim()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c.to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn slugs_match_github() {
        assert_eq!(super::slug("Overrides at every layer"), "overrides-at-every-layer");
        assert_eq!(super::slug("`nd new` and the README"), "nd-new-and-the-readme");
        assert_eq!(super::slug("Tier 1: no window"), "tier-1-no-window");
    }
}
