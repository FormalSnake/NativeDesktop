//! Links in the grid: OSC 8 hyperlinks come from libghostty; plain URLs are found in a row's
//! text here. A URL runs from a known scheme to the first blank or quote, minus trailing
//! punctuation and closing brackets it did not open.

use crate::frame::{Attrs, Row};

/// A link under the pointer: its target and the cells it covers on one row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub url: String,
    pub row: u16,
    /// First column, and one past the last.
    pub start: u16,
    pub end: u16,
}

const SCHEMES: [&str; 7] = ["https://", "http://", "file://", "ftp://", "ssh://", "git://", "mailto:"];

/// The URL covering column `col` of `row`, with the columns it spans.
pub fn url_at(row: &Row, col: u16) -> Option<(String, u16, u16)> {
    // The row as characters, each with the column it starts in.
    let mut chars: Vec<(char, u16)> = Vec::with_capacity(row.cells.len());
    for (i, cell) in row.cells.iter().enumerate() {
        if cell.attrs.contains(Attrs::SPACER) {
            continue;
        }
        let at = i as u16;
        if cell.is_blank() {
            chars.push((' ', at));
        } else {
            chars.extend(row.grapheme(cell).chars().map(|c| (c, at)));
        }
    }
    find_urls(&chars.iter().map(|(c, _)| *c).collect::<String>()).into_iter().find_map(|(start, end)| {
        let first = chars[start].1;
        let last = chars[end - 1].1;
        let width = row.cells.get(usize::from(last)).map_or(1, |c| c.width());
        (first..last + width).contains(&col).then(|| {
            let url: String = chars[start..end].iter().map(|(c, _)| *c).collect();
            (url, first, last + width)
        })
    })
}

/// Every URL in `text`, as char index ranges.
pub fn find_urls(text: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let Some(scheme) = SCHEMES.iter().find(|s| starts_with(&chars[i..], s)) else {
            i += 1;
            continue;
        };
        // A scheme glued to a word ("xhttps://") is not a link start.
        if i > 0 && chars[i - 1].is_alphanumeric() {
            i += 1;
            continue;
        }
        let body = i + scheme.chars().count();
        let mut end = body;
        while end < chars.len() && is_url_char(chars[end]) {
            end += 1;
        }
        let end = trim_end(&chars[i..end]) + i;
        if end > body {
            out.push((i, end));
        }
        i = end.max(i + 1);
    }
    out
}

fn starts_with(chars: &[char], prefix: &str) -> bool {
    let mut it = chars.iter();
    prefix.chars().all(|p| it.next().is_some_and(|c| c.eq_ignore_ascii_case(&p)))
}

fn is_url_char(c: char) -> bool {
    !c.is_whitespace() && !c.is_control() && !matches!(c, '"' | '<' | '>' | '`' | '\'' | '\u{a0}')
}

/// Length of `url` once trailing punctuation and unbalanced closers are dropped.
fn trim_end(url: &[char]) -> usize {
    let mut end = url.len();
    while end > 0 {
        let c = url[end - 1];
        let drop = match c {
            '.' | ',' | ':' | ';' | '!' | '?' => true,
            ')' | ']' | '}' => {
                let open = match c {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                let opens = url[..end].iter().filter(|x| **x == open).count();
                let closes = url[..end].iter().filter(|x| **x == c).count();
                closes > opens
            }
            _ => false,
        };
        if !drop {
            break;
        }
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        find_urls(text).into_iter().map(|(s, e)| chars[s..e].iter().collect()).collect()
    }

    #[test]
    fn trims_punctuation_and_unbalanced_closers() {
        assert_eq!(urls("see https://example.com/a."), ["https://example.com/a"]);
        assert_eq!(urls("(https://example.com/a)"), ["https://example.com/a"]);
        assert_eq!(
            urls("https://en.wikipedia.org/wiki/Rust_(language)"),
            ["https://en.wikipedia.org/wiki/Rust_(language)"]
        );
        assert_eq!(urls("x https://a.b/c, and file:///tmp/x"), ["https://a.b/c", "file:///tmp/x"]);
        assert_eq!(urls("'https://quoted.example'"), ["https://quoted.example"]);
        assert!(urls("nohttps://x").is_empty());
        assert!(urls("https://").is_empty());
    }
}
