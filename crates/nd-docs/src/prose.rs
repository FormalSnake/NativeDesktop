//! The prose check over docs pages and the templates `nd new` writes: the tells that make text
//! read as generated. Code blocks and inline code are exempt.

use crate::model::Page;

/// Words that sell instead of saying something.
pub const BANNED_WORDS: &[&str] = &[
    "comprehensive",
    "robust",
    "seamless",
    "seamlessly",
    "powerful",
    "leverage",
    "leverages",
    "delve",
    "elevate",
    "streamline",
    "significantly",
    "effortless",
    "effortlessly",
    "cutting-edge",
    "blazing",
];

/// Phrases with no content.
pub const BANNED_PHRASES: &[&str] =
    &["it's worth noting", "it is worth noting", "in summary", "let's dive in", "smoke test", "in conclusion"];

/// `(line text, problem)` for each hit in `text`, skipping fenced code and inline code.
pub fn check_text(text: &str) -> Vec<(usize, String)> {
    let mut hits = Vec::new();
    let mut fence = false;
    for (n, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = !fence;
            continue;
        }
        if fence {
            continue;
        }
        let prose = strip_inline_code(line);
        for problem in check_line(&prose) {
            hits.push((n + 1, problem));
        }
    }
    hits
}

fn strip_inline_code(line: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in line.chars() {
        if c == '`' {
            inside = !inside;
        } else if !inside {
            out.push(c);
        }
    }
    out
}

pub fn check_line(prose: &str) -> Vec<String> {
    let mut problems = Vec::new();
    if prose.contains('\u{2014}') {
        problems.push("em dash: use a comma, colon, parentheses or two sentences".into());
    }
    if prose.contains(" \u{2013} ") {
        problems.push("spaced en dash: use a comma, colon, parentheses or two sentences".into());
    }
    let lower = prose.to_lowercase();
    for word in lower.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '\'')) {
        if BANNED_WORDS.contains(&word) {
            problems.push(format!("`{word}`: say what it does instead"));
        }
    }
    for phrase in BANNED_PHRASES {
        if lower.contains(phrase) {
            problems.push(format!("`{phrase}`: cut it"));
        }
    }
    problems
}

/// Problems in a page's own prose (generated parts included).
pub fn check_page(page: &Page) -> Vec<String> {
    let mut out = Vec::new();
    for (line, problem) in check_text(&page.markdown) {
        out.push(format!("docs/content/{}.md (resolved line {line}): {problem}", page.id));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_the_tells() {
        assert_eq!(check_text("A robust and seamless way.").len(), 2);
        assert_eq!(check_text("one \u{2014} two").len(), 1);
        assert!(check_text("`robust` in code is fine\n```\nrobust\n```").is_empty());
        assert!(check_text("A plain sentence.").is_empty());
    }
}
