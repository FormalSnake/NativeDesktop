//! Playwright-shaped selector strings, the old repo's grammar unchanged. A selector is one or
//! more parts joined by `>>`; an engine part searches inside the previous part's matches,
//! positional and filter parts refine the current match set in place.
//!
//! ```text
//! testid=save-btn
//! role=button[name="Save"][exact] >> nth=1
//! type=Button >> has-text=Save >> first
//! ```
//!
//! Engines: `testid=`, `role=` (attributes `[name=..]`, `[exact]`, `[checked]`, `[disabled]`),
//! `text=`, `label=`, `placeholder=`, `type=`, `has-text=`, `has=(..)`, `has-not=(..)`,
//! `and=(..)`, `nth=N`, `first`, `last`. Text is a case-insensitive substring, `"quoted"` for
//! exact (whitespace-normalised), `/regex/flags` for a regular expression.

use std::fmt;

use regex::Regex;

use crate::Error;

#[derive(Clone, Debug)]
pub enum TextMatch {
    Substring(String),
    Exact(String),
    Regex(Regex),
}

impl PartialEq for TextMatch {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (TextMatch::Substring(a), TextMatch::Substring(b)) | (TextMatch::Exact(a), TextMatch::Exact(b)) => a == b,
            (TextMatch::Regex(a), TextMatch::Regex(b)) => a.as_str() == b.as_str(),
            _ => false,
        }
    }
}

fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl TextMatch {
    pub fn matches(&self, actual: &str) -> bool {
        match self {
            TextMatch::Substring(want) => normalize(actual).to_lowercase().contains(&normalize(want).to_lowercase()),
            TextMatch::Exact(want) => normalize(actual) == normalize(want),
            TextMatch::Regex(re) => re.is_match(actual),
        }
    }

    /// The literal a failing locator ranks candidates against.
    pub fn literal(&self) -> &str {
        match self {
            TextMatch::Substring(s) | TextMatch::Exact(s) => s,
            TextMatch::Regex(re) => re.as_str(),
        }
    }

    fn exact(self) -> Self {
        match self {
            TextMatch::Substring(s) => TextMatch::Exact(s),
            other => other,
        }
    }
}

impl fmt::Display for TextMatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TextMatch::Substring(s) => f.write_str(s),
            TextMatch::Exact(s) => write!(f, "{s:?}"),
            TextMatch::Regex(re) => write!(f, "/{}/", re.as_str()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    TestId(String),
    Role { role: String, name: Option<TextMatch>, checked: Option<bool>, disabled: Option<bool> },
    Text(TextMatch),
    Label(TextMatch),
    Placeholder(TextMatch),
    Type(String),
    Nth(i64),
    HasText(TextMatch),
    Has(Vec<Part>),
    HasNot(Vec<Part>),
    And(Vec<Part>),
}

fn err(selector: &str, what: impl fmt::Display) -> Error {
    Error::Selector(format!("selector \"{selector}\": {what}"))
}

/// Splits on top-level `>>`, ignoring separators inside quotes, brackets, parens and regexes.
fn split_parts(selector: &str) -> Vec<String> {
    let chars: Vec<char> = selector.chars().collect();
    let mut out = Vec::new();
    let (mut depth, mut quote, mut in_regex, mut start) = (0i32, None::<char>, false, 0usize);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            if c == '\\' {
                i += 1;
            } else if c == q {
                quote = None;
            }
        } else if in_regex {
            if c == '\\' {
                i += 1;
            } else if c == '/' {
                in_regex = false;
            }
        } else if c == '"' || c == '\'' {
            quote = Some(c);
        } else if c == '/' && i > 0 && chars[i - 1] == '=' {
            in_regex = true;
        } else if c == '[' || c == '(' {
            depth += 1;
        } else if c == ']' || c == ')' {
            depth -= 1;
        } else if depth == 0 && c == '>' && chars.get(i + 1) == Some(&'>') {
            out.push(chars[start..i].iter().collect());
            i += 1;
            start = i + 1;
        }
        i += 1;
    }
    out.push(chars[start.min(chars.len())..].iter().collect());
    out.into_iter().map(|p: String| p.trim().to_owned()).filter(|p| !p.is_empty()).collect()
}

fn unquote(s: &str) -> String {
    let mut chars = s.chars();
    let q = chars.next();
    let inner: Vec<char> = chars.collect();
    let inner = &inner[..inner.len().saturating_sub(1)];
    let mut out = String::new();
    let mut i = 0;
    while i < inner.len() {
        let c = inner[i];
        if c == '\\' {
            i += 1;
            match inner.get(i) {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(*other),
                None => {}
            }
        } else if Some(c) != q {
            out.push(c);
        }
        i += 1;
    }
    out
}

fn parse_regex(body: &str, selector: &str) -> Result<Option<TextMatch>, Error> {
    let Some(rest) = body.strip_prefix('/') else { return Ok(None) };
    let Some(end) = rest.rfind('/') else { return Ok(None) };
    let (pattern, flags) = (&rest[..end], &rest[end + 1..]);
    let mut prefix = String::new();
    for flag in flags.chars() {
        match flag {
            'i' | 'm' | 's' | 'x' => prefix.push(flag),
            other => return Err(err(selector, format!("regex flag {other} is not supported"))),
        }
    }
    let pattern = if prefix.is_empty() { pattern.to_owned() } else { format!("(?{prefix}){pattern}") };
    Regex::new(&pattern).map(|re| Some(TextMatch::Regex(re))).map_err(|e| err(selector, e))
}

fn parse_text(body: &str, selector: &str) -> Result<TextMatch, Error> {
    let body = body.trim();
    if let Some(re) = parse_regex(body, selector)? {
        return Ok(re);
    }
    let quoted = body.len() >= 2
        && (body.starts_with('"') || body.starts_with('\''))
        && body.ends_with(body.chars().next().unwrap_or('"'));
    Ok(if quoted { TextMatch::Exact(unquote(body)) } else { TextMatch::Substring(body.to_owned()) })
}

fn parse_role(body: &str, selector: &str) -> Result<Part, Error> {
    let bracket = body.find('[').unwrap_or(body.len());
    let role = body[..bracket].trim().to_owned();
    if role.is_empty() {
        return Err(err(selector, "role= needs a role name"));
    }
    let (mut name, mut exact, mut checked, mut disabled) = (None, false, None, None);
    let mut rest = &body[bracket..];
    while !rest.is_empty() {
        let Some(inner) = rest.strip_prefix('[') else {
            return Err(err(selector, format!("malformed attributes \"{rest}\"")));
        };
        // A quoted or regex value may contain `]`.
        let close = attr_end(inner).ok_or_else(|| err(selector, format!("unclosed attribute \"[{inner}\"")))?;
        let attr = &inner[..close];
        rest = &inner[close + 1..];
        let (key, value) = match attr.split_once('=') {
            Some((k, v)) => (k.trim(), Some(v)),
            None => (attr.trim(), None),
        };
        let flag = || value.is_none_or(|v| v.trim() != "false");
        match key {
            "name" => name = Some(parse_text(value.unwrap_or_default(), selector)?),
            "exact" => exact = flag(),
            "checked" => checked = Some(flag()),
            "disabled" => disabled = Some(flag()),
            other => return Err(err(selector, format!("unknown role attribute [{other}]"))),
        }
    }
    let name = if exact { name.map(TextMatch::exact) } else { name };
    Ok(Part::Role { role, name, checked, disabled })
}

fn attr_end(s: &str) -> Option<usize> {
    let mut quote = None;
    let mut in_regex = false;
    let bytes: Vec<char> = s.chars().collect();
    let mut offset = 0;
    for (i, c) in bytes.iter().enumerate() {
        if let Some(q) = quote {
            if *c == q && bytes.get(i.wrapping_sub(1)) != Some(&'\\') {
                quote = None;
            }
        } else if in_regex {
            if *c == '/' {
                in_regex = false;
            }
        } else if *c == '"' || *c == '\'' {
            quote = Some(*c);
        } else if *c == '/' && i > 0 && bytes[i - 1] == '=' {
            in_regex = true;
        } else if *c == ']' {
            return Some(offset);
        }
        offset += c.len_utf8();
    }
    None
}

fn parse_nested(body: &str, selector: &str) -> Result<Vec<Part>, Error> {
    let body = body.trim();
    let inner = body.strip_prefix('(').and_then(|b| b.strip_suffix(')')).unwrap_or(body);
    let parts = parse(inner)?;
    if parts.is_empty() {
        return Err(err(selector, "empty nested selector"));
    }
    Ok(parts)
}

pub fn parse(selector: &str) -> Result<Vec<Part>, Error> {
    let mut out = Vec::new();
    for raw in split_parts(selector) {
        match raw.as_str() {
            "first" => {
                out.push(Part::Nth(0));
                continue;
            }
            "last" => {
                out.push(Part::Nth(-1));
                continue;
            }
            _ => {}
        }
        let Some((engine, body)) = raw.split_once('=') else {
            return Err(err(selector, format!("part \"{raw}\" has no engine (want engine=value)")));
        };
        let body = body.trim();
        out.push(match engine.trim().to_lowercase().as_str() {
            "nth" => Part::Nth(body.parse().map_err(|_| err(selector, format!("nth={body} is not an integer")))?),
            "testid" | "test-id" => Part::TestId(if body.starts_with('"') || body.starts_with('\'') {
                unquote(body)
            } else {
                body.to_owned()
            }),
            "type" => Part::Type(body.to_owned()),
            "role" => parse_role(body, selector)?,
            "text" => Part::Text(parse_text(body, selector)?),
            "label" => Part::Label(parse_text(body, selector)?),
            "placeholder" => Part::Placeholder(parse_text(body, selector)?),
            "has-text" => Part::HasText(parse_text(body, selector)?),
            "has" => Part::Has(parse_nested(body, selector)?),
            "has-not" => Part::HasNot(parse_nested(body, selector)?),
            "and" => Part::And(parse_nested(body, selector)?),
            other => return Err(err(selector, format!("unknown engine \"{other}\""))),
        });
    }
    Ok(out)
}

fn format_part(part: &Part) -> String {
    match part {
        Part::TestId(v) => format!("testid={v}"),
        Part::Type(v) => format!("type={v}"),
        Part::Nth(i) => format!("nth={i}"),
        Part::Role { role, name, checked, disabled } => {
            let mut out = format!("role={role}");
            if let Some(name) = name {
                out.push_str(&format!("[name={name}]"));
            }
            for (key, value) in [("checked", checked), ("disabled", disabled)] {
                match value {
                    Some(true) => out.push_str(&format!("[{key}]")),
                    Some(false) => out.push_str(&format!("[{key}=false]")),
                    None => {}
                }
            }
            out
        }
        Part::Text(t) => format!("text={t}"),
        Part::Label(t) => format!("label={t}"),
        Part::Placeholder(t) => format!("placeholder={t}"),
        Part::HasText(t) => format!("has-text={t}"),
        Part::Has(p) => format!("has=({})", format(p)),
        Part::HasNot(p) => format!("has-not=({})", format(p)),
        Part::And(p) => format!("and=({})", format(p)),
    }
}

/// Prints parts back as a selector; `parse(format(p)) == p`.
pub fn format(parts: &[Part]) -> String {
    parts.iter().map(format_part).collect::<Vec<_>>().join(" >> ")
}

/// The literal the last engine part was reaching for, for ranking near misses.
pub fn intended_name(parts: &[Part]) -> Option<String> {
    parts.iter().rev().find_map(|p| match p {
        Part::TestId(v) | Part::Type(v) => Some(v.clone()),
        Part::Role { role, name, .. } => Some(name.as_ref().map_or(role.clone(), |n| n.literal().to_owned())),
        Part::Text(t) | Part::Label(t) | Part::Placeholder(t) | Part::HasText(t) => Some(t.literal().to_owned()),
        _ => None,
    })
}
