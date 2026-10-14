//! Code cut from the example apps by anchor comments, so a page never carries a pasted copy:
//!
//! ```text
//! // docs:begin counter
//! pub struct Counter { .. }
//! // docs:end counter
//! ```
//!
//! A page asks for it with `<!-- docs:snippet examples/hello/src/lib.rs#counter -->`; without
//! `#anchor` the whole file is shown. Anchors may nest and overlap; every anchor line is left out
//! of every snippet, and the cut is dedented.

use std::path::Path;

use anyhow::{Context, Result, bail};

pub const BEGIN: &str = "docs:begin";
pub const END: &str = "docs:end";

#[derive(Clone, Debug, PartialEq)]
pub struct Snippet {
    pub code: String,
    /// 1-based line of the first line kept.
    pub line: usize,
}

fn anchor_of<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let rest = line.trim_start();
    let rest = rest.strip_prefix("//").or_else(|| rest.strip_prefix('#'))?;
    rest.trim().strip_prefix(marker).map(str::trim).filter(|a| !a.is_empty())
}

pub fn is_anchor_line(line: &str) -> bool {
    anchor_of(line, BEGIN).is_some() || anchor_of(line, END).is_some()
}

/// The anchors `source` defines, in order.
pub fn anchors(source: &str) -> Vec<String> {
    source.lines().filter_map(|l| anchor_of(l, BEGIN)).map(str::to_owned).collect()
}

pub fn extract(source: &str, anchor: Option<&str>) -> Result<Snippet> {
    let lines: Vec<&str> = source.lines().collect();
    let (from, to) = match anchor {
        None => (0, lines.len()),
        Some(name) => {
            let begin = lines
                .iter()
                .position(|l| anchor_of(l, BEGIN) == Some(name))
                .with_context(|| format!("no `// {BEGIN} {name}`"))?;
            let end = lines[begin + 1..]
                .iter()
                .position(|l| anchor_of(l, END) == Some(name))
                .map(|i| begin + 1 + i)
                .with_context(|| format!("`// {BEGIN} {name}` has no `// {END} {name}`"))?;
            (begin + 1, end)
        }
    };
    let kept: Vec<(usize, &str)> =
        lines[from..to].iter().enumerate().map(|(i, l)| (from + i, *l)).filter(|(_, l)| !is_anchor_line(l)).collect();
    let Some(first) = kept.iter().position(|(_, l)| !l.trim().is_empty()) else {
        bail!("snippet `{}` is empty", anchor.unwrap_or("<file>"));
    };
    let last = kept.iter().rposition(|(_, l)| !l.trim().is_empty()).unwrap_or(first);
    let kept = &kept[first..=last];
    let indent = kept
        .iter()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(_, l)| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let code = kept.iter().map(|(_, l)| l.get(indent..).unwrap_or("").trim_end()).collect::<Vec<_>>().join("\n");
    Ok(Snippet { code, line: kept[0].0 + 1 })
}

/// `path#anchor`, the path relative to the repository root.
pub fn load(repo: &Path, spec: &str) -> Result<(String, Option<String>, Snippet)> {
    let (path, anchor) = match spec.split_once('#') {
        Some((p, a)) => (p.trim(), Some(a.trim().to_owned())),
        None => (spec.trim(), None),
    };
    let text = std::fs::read_to_string(repo.join(path)).with_context(|| format!("snippet source {path}"))?;
    let snippet = extract(&text, anchor.as_deref()).with_context(|| format!("snippet {spec}"))?;
    Ok((path.to_owned(), anchor, snippet))
}

pub fn lang_of(path: &str) -> &'static str {
    match Path::new(path).extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("toml") => "toml",
        Some("swift") => "swift",
        Some("nix") => "nix",
        Some("sh") => "sh",
        Some("json") => "json",
        Some("md") => "markdown",
        _ => "text",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "fn a() {}\n// docs:begin outer\nimpl X {\n    // docs:begin inner\n    fn b() {}\n    // docs:end inner\n}\n// docs:end outer\n";

    #[test]
    fn cuts_and_dedents() {
        let inner = extract(SRC, Some("inner")).unwrap();
        assert_eq!(inner.code, "fn b() {}");
        assert_eq!(inner.line, 5);
        let outer = extract(SRC, Some("outer")).unwrap();
        assert_eq!(outer.code, "impl X {\n    fn b() {}\n}");
        assert_eq!(extract(SRC, None).unwrap().code, "fn a() {}\nimpl X {\n    fn b() {}\n}");
        assert_eq!(anchors(SRC), ["outer", "inner"]);
    }

    #[test]
    fn missing_anchor_is_an_error() {
        assert!(extract(SRC, Some("nope")).is_err());
        assert!(extract("// docs:begin x\nfn a() {}\n", Some("x")).is_err());
    }
}
