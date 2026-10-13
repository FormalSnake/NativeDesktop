//! ARCHITECTURE.md 3.10: the AppKit mapping holds no system metrics, colours, radii or materials
//! as constants. It reads them from the OS (semantic `NSColor`s, text styles, `fittingSize`) or
//! lets the control draw itself, so a new macOS release restyles apps with no framework change.
//! This scans the backend's Rust and Swift sources for the shapes such constants take: colour
//! literals, metric-named numeric constants, and numeric literals passed to geometry setters and
//! SwiftUI layout modifiers. Values that come from the app (its own style, props) are variables
//! and never match.
//!
//! A line that must keep a literal (a protocol constant, a ratio of an OS-read value) carries
//! `nd-lint: allow` with its reason on the same line.

use std::path::{Path, PathBuf};

/// Each pattern names one way a hardcoded system value shows up.
const PATTERNS: &[(&str, &str)] = &[
    (
        r"colorWith(SRGB|Calibrated|Device|Generic)?(Red|White|Hue)[^)]*\(\s*-?[0-9.]+",
        "a colour built from literal components",
    ),
    (r"(NSColor|Color|UIColor)\s*\(\s*(red|white|hue|srgbRed)\s*:\s*[0-9.]", "a SwiftUI or AppKit colour literal"),
    (r##""#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?""##, "a hex colour string"),
    (r"\b(rgb|rgba|hsla)\(\s*(0x)?[0-9]", "a gpui colour literal"),
    (
        r"const\s+[A-Z0-9_]*(HEIGHT|WIDTH|RADIUS|INSET|MARGIN|PADDING|SPACING|THICKNESS|GAP|SIZE)[A-Z0-9_]*\s*:\s*(f32|f64|CGFloat|isize|usize|i32|u32)\s*=\s*[0-9]",
        "a metric constant",
    ),
    (
        r"\.set(CornerRadius|BorderWidth|MinimumThickness|MaximumThickness|Spacing|FrameSize|ContentMinSize)\(\s*-?([1-9][0-9.]*|0\.[0-9]*[1-9][0-9]*)\s*[),]",
        "a geometry setter given a literal",
    ),
    (r"NSSize::new\(\s*[1-9][0-9.]*\s*,\s*[0-9]", "a literal size"),
    (r"\.(padding|cornerRadius|frame)\(\s*(width:\s*|height:\s*)?[0-9]", "a SwiftUI layout literal"),
];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if !path.ends_with(".build") {
                sources(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs" || e == "swift") {
            out.push(path);
        }
    }
}

/// Matches `pattern` against `line` with a tiny regex subset: the shapes above only use
/// literals, `\b`, `\s`, `[...]`, groups with `|`, and `? * +` on single atoms, so this stays
/// dependency free.
fn matches(pattern: &str, line: &str) -> bool {
    regex_lite_match(pattern, line)
}

mod engine {
    //! Backtracking matcher over the subset `matches` documents.

    #[derive(Debug, Clone)]
    pub enum Node {
        Char(char),
        Any,
        Class(Vec<(char, char)>, bool),
        Digit,
        Space,
        Word,
        Boundary,
        Group(Vec<Vec<Node>>),
        Repeat(Box<Node>, usize, usize),
    }

    pub fn parse(p: &str) -> Vec<Vec<Node>> {
        let chars: Vec<char> = p.chars().collect();
        let mut i = 0;
        let alts = alternation(&chars, &mut i);
        assert_eq!(i, chars.len(), "unbalanced pattern {p}");
        alts
    }

    fn alternation(c: &[char], i: &mut usize) -> Vec<Vec<Node>> {
        let mut alts = vec![Vec::new()];
        while *i < c.len() {
            match c[*i] {
                ')' => break,
                '|' => {
                    *i += 1;
                    alts.push(Vec::new());
                }
                _ => {
                    let atom = atom(c, i);
                    let atom = quantify(c, i, atom);
                    alts.last_mut().unwrap().push(atom);
                }
            }
        }
        alts
    }

    fn quantify(c: &[char], i: &mut usize, atom: Node) -> Node {
        let (min, max) = match c.get(*i) {
            Some('?') => (0, 1),
            Some('*') => (0, usize::MAX),
            Some('+') => (1, usize::MAX),
            Some('{') => {
                let end = c[*i..].iter().position(|ch| *ch == '}').unwrap() + *i;
                let n: usize = c[*i + 1..end].iter().collect::<String>().parse().unwrap();
                *i = end;
                (n, n)
            }
            _ => return atom,
        };
        *i += 1;
        Node::Repeat(Box::new(atom), min, max)
    }

    fn atom(c: &[char], i: &mut usize) -> Node {
        let ch = c[*i];
        *i += 1;
        match ch {
            '.' => Node::Any,
            '(' => {
                let alts = alternation(c, i);
                *i += 1;
                Node::Group(alts)
            }
            '[' => {
                let negate = c[*i] == '^';
                if negate {
                    *i += 1;
                }
                let mut ranges = Vec::new();
                while c[*i] != ']' {
                    let lo = c[*i];
                    if c.get(*i + 1) == Some(&'-') && c.get(*i + 2) != Some(&']') {
                        ranges.push((lo, c[*i + 2]));
                        *i += 3;
                    } else {
                        ranges.push((lo, lo));
                        *i += 1;
                    }
                }
                *i += 1;
                Node::Class(ranges, negate)
            }
            '\\' => {
                let e = c[*i];
                *i += 1;
                match e {
                    'd' => Node::Digit,
                    's' => Node::Space,
                    'w' => Node::Word,
                    'b' => Node::Boundary,
                    other => Node::Char(other),
                }
            }
            other => Node::Char(other),
        }
    }

    fn one(n: &Node, s: &[char], at: usize) -> bool {
        let Some(&ch) = s.get(at) else { return false };
        match n {
            Node::Char(c) => *c == ch,
            Node::Any => true,
            Node::Digit => ch.is_ascii_digit(),
            Node::Space => ch.is_whitespace(),
            Node::Word => ch.is_alphanumeric() || ch == '_',
            Node::Class(r, neg) => r.iter().any(|(lo, hi)| (*lo..=*hi).contains(&ch)) != *neg,
            _ => unreachable!(),
        }
    }

    /// End positions `seq` can match at, starting at `at`.
    pub fn run(seq: &[Node], s: &[char], at: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
        let Some((first, rest)) = seq.split_first() else { return k(at) };
        match first {
            Node::Boundary => {
                let word = |i: Option<&char>| i.is_some_and(|c| c.is_alphanumeric() || *c == '_');
                let before = at.checked_sub(1).and_then(|i| s.get(i));
                if word(before) != word(s.get(at)) { run(rest, s, at, k) } else { false }
            }
            Node::Group(alts) => alts.iter().any(|alt| run(alt, s, at, &mut |end| run(rest, s, end, k))),
            Node::Repeat(inner, min, max) => repeat(inner, *min, *max, 0, rest, s, at, k),
            atom => one(atom, s, at) && run(rest, s, at + 1, k),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn repeat(
        inner: &Node,
        min: usize,
        max: usize,
        count: usize,
        rest: &[Node],
        s: &[char],
        at: usize,
        k: &mut dyn FnMut(usize) -> bool,
    ) -> bool {
        if count < max {
            let more = run(std::slice::from_ref(inner), s, at, &mut |end| {
                end > at && repeat(inner, min, max, count + 1, rest, s, end, k)
            });
            if more {
                return true;
            }
        }
        count >= min && run(rest, s, at, k)
    }
}

fn regex_lite_match(pattern: &str, line: &str) -> bool {
    let alts = engine::parse(pattern);
    let s: Vec<char> = line.chars().collect();
    (0..=s.len()).any(|start| alts.iter().any(|alt| engine::run(alt, &s, start, &mut |_| true)))
}

fn violations() -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    sources(&root.join("src"), &mut files);
    sources(&root.join("swift"), &mut files);
    files.sort();
    let mut found = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("source");
        for (n, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") || line.contains("nd-lint: allow") {
                continue;
            }
            for (pattern, what) in PATTERNS {
                if matches(pattern, line) {
                    let rel = file.strip_prefix(root).unwrap_or(&file).display();
                    found.push(format!("{rel}:{}: {what}: {}", n + 1, line.trim()));
                }
            }
        }
    }
    found
}

mod gate_m3 {
    #[test]
    fn native_mappings_hold_no_system_literals() {
        let found = super::violations();
        assert!(
            found.is_empty(),
            "native mappings must read system values from the OS (ARCHITECTURE.md 3.10):\n{}",
            found.join("\n")
        );
    }
}

#[test]
fn the_lint_catches_each_shape() {
    let bad = [
        "NSColor::colorWithSRGBRed_green_blue_alpha(0.2, 0.4, 1.0, 1.0)",
        "Color(red: 0.1, green: 0.2, blue: 0.3)",
        r##"let accent = "#3d6bff";"##,
        "const TOOLBAR_HEIGHT: f64 = 52.0;",
        "view.setCornerRadius(12.0);",
        "layer.setBorderWidth(0.5);",
        "item.setMinimumThickness(200.);",
        "NSSize::new(16., 16.)",
        ".padding(8)",
        ".frame(width: 640)",
        "div().bg(rgb(0x3366ff))",
    ];
    for line in bad {
        assert!(PATTERNS.iter().any(|(p, _)| matches(p, line)), "not caught: {line}");
    }
    let good = [
        "NSColor::colorWithSRGBRed_green_blue_alpha(r.into(), g.into(), b.into(), a.into())",
        "glass.setCornerRadius(r);",
        "let size = probe.fittingSize();",
        ".padding()",
        "NSColor::controlAccentColor()",
        "item.setMinimumThickness(min.into());",
        "None => layer.setBorderWidth(0.),",
    ];
    for line in good {
        assert!(!PATTERNS.iter().any(|(p, _)| matches(p, line)), "false positive: {line}");
    }
}
