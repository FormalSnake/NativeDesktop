//! A page file to [`Page`]: TOML front matter between `+++` lines, then markdown with directives
//! on lines of their own (outside code fences):
//!
//! - `<!-- docs:snippet examples/hello/src/lib.rs#counter -->`: code cut from an example.
//! - `<!-- docs:example hello -->`: the four-tab embed of an example.
//! - `<!-- docs:gallery -->`: every example, each with its embed.
//! - `<!-- docs:components -->`: the component reference generated from `nd-components`.
//! - `<!-- docs:include crates/nd-cli/templates/docs/agents/donts.md -->`: another markdown file
//!   inlined as prose (its own leading `# Title` dropped), so the agent docs `nd new` copies into
//!   a project and the docs pages are one text. Links in it resolve from the including page.
//!
//! The directives are HTML comments so the raw file still reads well on GitHub.

use std::path::Path;

use anyhow::{Context, Result, bail};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::Deserialize;

use crate::examples::Example;
use crate::model::{Block, Inline, Link, Page, SourceRef, plain, slug};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Front {
    title: Option<String>,
    #[serde(default)]
    description: String,
}

pub struct PageContext<'a> {
    pub repo: &'a Path,
    pub examples: &'a [Example],
    /// The generated component reference, as markdown.
    pub components: &'a dyn Fn() -> Result<String>,
}

pub fn directive(line: &str) -> Option<(&str, &str)> {
    let inner = line.trim().strip_prefix("<!--")?.strip_suffix("-->")?.trim();
    let rest = inner.strip_prefix("docs:")?;
    Some(rest.split_once(char::is_whitespace).map_or((rest, ""), |(k, v)| (k, v.trim())))
}

/// Parses `text`, the file of page `id`.
pub fn page(id: &str, text: &str, cx: &PageContext<'_>) -> Result<Page> {
    let (front, body) = split_front(text).with_context(|| format!("{id}: front matter"))?;
    let base = match id.rsplit_once('/') {
        Some((dir, _)) => format!("docs/content/{dir}"),
        None => "docs/content".to_owned(),
    };
    let mut blocks = Vec::new();
    let mut markdown = String::new();
    let mut chunk = String::new();
    let mut fence: Option<String> = None;
    let flush = |chunk: &mut String, blocks: &mut Vec<Block>, markdown: &mut String| {
        if !chunk.trim().is_empty() {
            blocks.extend(parse_blocks(chunk, id, &base));
            markdown.push_str(chunk);
        }
        chunk.clear();
    };
    for line in body.lines() {
        let trimmed = line.trim_start();
        if let Some(open) = &fence {
            if trimmed.starts_with(open.as_str()) {
                fence = None;
            }
            chunk.push_str(line);
            chunk.push('\n');
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(trimmed.chars().take_while(|c| *c == '`' || *c == '~').collect());
            chunk.push_str(line);
            chunk.push('\n');
            continue;
        }
        let Some((kind, arg)) = directive(line) else {
            chunk.push_str(line);
            chunk.push('\n');
            continue;
        };
        flush(&mut chunk, &mut blocks, &mut markdown);
        match kind {
            "snippet" => {
                let (path, anchor, snippet) = crate::snippets::load(cx.repo, arg).with_context(|| id.to_string())?;
                let lang = crate::snippets::lang_of(&path);
                let fence = if snippet.code.contains("```") { "````" } else { "```" };
                markdown.push_str(&format!("{fence}{lang}\n{}\n{fence}\n\n", snippet.code));
                markdown.push_str(&format!(
                    "<sub>From [{path}]({}), line {}.</sub>\n\n",
                    relative_to(&base, &path),
                    snippet.line
                ));
                blocks.push(Block::Code {
                    lang: lang.to_owned(),
                    code: snippet.code,
                    source: Some(SourceRef { path, anchor, line: snippet.line }),
                });
            }
            "example" => {
                let example = cx
                    .examples
                    .iter()
                    .find(|e| e.name == arg)
                    .with_context(|| format!("{id}: `docs:example {arg}` is not in docs/examples.toml"))?;
                markdown.push_str(&example_markdown(example, &base));
                blocks.push(Block::Example(example.name.clone()));
            }
            "gallery" => {
                for example in cx.examples {
                    let section = format!("### {}\n\n{}\n\n", example.title, example.summary);
                    blocks.extend(parse_blocks(&section, id, &base));
                    markdown.push_str(&section);
                    markdown.push_str(&example_markdown(example, &base));
                    blocks.push(Block::Example(example.name.clone()));
                }
            }
            "include" => {
                let text = std::fs::read_to_string(cx.repo.join(arg))
                    .with_context(|| format!("{id}: `docs:include {arg}`"))?;
                let text = match text.strip_prefix("# ") {
                    Some(rest) => rest.split_once('\n').map_or("", |(_, body)| body).trim_start().to_owned(),
                    None => text,
                };
                blocks.extend(parse_blocks(&text, id, &base));
                markdown.push_str(&text);
                markdown.push('\n');
            }
            "components" => {
                let generated = (cx.components)()?;
                blocks.extend(parse_blocks(&generated, id, &base));
                markdown.push_str(&generated);
                markdown.push('\n');
            }
            other => bail!("{id}: unknown directive `docs:{other}`"),
        }
    }
    if fence.is_some() {
        bail!("{id}: unclosed code fence");
    }
    flush(&mut chunk, &mut blocks, &mut markdown);

    let leading_h1 = match blocks.first() {
        Some(Block::Heading { level: 1, text, .. }) => Some(plain(text)),
        _ => None,
    };
    if leading_h1.is_some() {
        blocks.remove(0);
    }
    let title = front.title.or(leading_h1).with_context(|| format!("{id}: no title (front matter or `# Title`)"))?;
    // The exported file always opens with its title, for readers of the raw markdown.
    let markdown = if markdown.trim_start().starts_with("# ") {
        markdown.trim_start().to_owned()
    } else {
        format!("# {title}\n\n{}", markdown.trim_start())
    };
    Ok(Page { id: id.to_owned(), title, description: front.description, blocks, markdown })
}

fn example_markdown(example: &Example, base: &str) -> String {
    format!(
        "> **Example: {}.** Source in [examples/{}]({}). Its page on the docs site shows it captured \
         on macOS, Linux and Windows.\n\n",
        example.title,
        example.name,
        relative_to(base, &format!("examples/{}", example.name))
    )
}

fn split_front(text: &str) -> Result<(Front, &str)> {
    let Some(rest) = text.strip_prefix("+++\n") else {
        return Ok((Front::default(), text));
    };
    let end = rest.find("\n+++\n").context("front matter has no closing +++")?;
    Ok((toml::from_str(&rest[..end])?, &rest[end + 5..]))
}

/// `path` (relative to the repository root) as a link from a file in `base`.
pub fn relative_to(base: &str, path: &str) -> String {
    let depth = base.split('/').filter(|s| !s.is_empty()).count();
    format!("{}{path}", "../".repeat(depth))
}

/// Joins `relative` onto `base` (both `/`-separated, repository-relative), resolving `..`.
/// `None` when it climbs out of the repository.
pub fn join(base: &str, relative: &str) -> Option<String> {
    let mut parts: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// A link target as written in page `id` (whose file sits in `base`).
pub fn classify(dest: &str, id: &str, base: &str) -> Link {
    if dest.contains("://") || dest.starts_with("mailto:") {
        return Link::External(dest.to_owned());
    }
    let (path, anchor) = match dest.split_once('#') {
        Some((p, a)) => (p, Some(a.to_owned())),
        None => (dest, None),
    };
    if path.is_empty() {
        return Link::Page { id: id.to_owned(), anchor };
    }
    let Some(target) = join(base, path) else {
        return Link::Repo(path.to_owned());
    };
    match target.strip_prefix("docs/content/").and_then(|p| p.strip_suffix(".md")) {
        Some(page) => Link::Page { id: page.to_owned(), anchor },
        None => Link::Repo(target),
    }
}

#[derive(Default)]
struct Style {
    strong: u32,
    emph: u32,
    link: Vec<Link>,
}

enum Frame {
    Blocks(Vec<Block>),
    Item(Vec<Block>),
    List { ordered: bool, items: Vec<Vec<Block>> },
    Inlines(Vec<Inline>),
    Table { header: Vec<Vec<Inline>>, rows: Vec<Vec<Vec<Inline>>>, row: Vec<Vec<Inline>>, in_head: bool },
}

/// Markdown to blocks. Raw HTML is dropped (it only carries `<sub>` notes and comments).
pub fn parse_blocks(text: &str, id: &str, base: &str) -> Vec<Block> {
    let mut stack: Vec<Frame> = vec![Frame::Blocks(Vec::new())];
    let mut style = Style::default();
    let mut code: Option<(String, String)> = None;
    let push_block = |stack: &mut Vec<Frame>, block: Block| {
        if let Some(Frame::Blocks(b) | Frame::Item(b)) = stack.last_mut() {
            b.push(block);
        }
    };
    let push_inline = |stack: &mut Vec<Frame>, style: &Style, text: &str, code: bool| {
        let run = Inline {
            text: text.to_owned(),
            code,
            strong: style.strong > 0,
            emph: style.emph > 0,
            link: style.link.last().cloned(),
        };
        // Tight list items have no paragraph: open one.
        if let Some(Frame::Item(_)) = stack.last() {
            stack.push(Frame::Inlines(Vec::new()));
        }
        if let Some(Frame::Inlines(runs)) = stack.last_mut() {
            match runs.last_mut() {
                Some(last)
                    if !code
                        && !last.code
                        && last.strong == run.strong
                        && last.emph == run.emph
                        && last.link == run.link =>
                {
                    last.text.push_str(&run.text)
                }
                _ => runs.push(run),
            }
        }
    };
    let close_loose = |stack: &mut Vec<Frame>| {
        if let Some(Frame::Inlines(_)) = stack.last()
            && matches!(stack.get(stack.len().wrapping_sub(2)), Some(Frame::Item(_)))
            && let Some(Frame::Inlines(runs)) = stack.pop()
            && let Some(Frame::Item(b)) = stack.last_mut()
        {
            b.push(Block::Paragraph(runs));
        }
    };
    for event in Parser::new_ext(text, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Start(Tag::Paragraph | Tag::Heading { .. }) => stack.push(Frame::Inlines(Vec::new())),
            Event::End(TagEnd::Paragraph) => {
                if let Some(Frame::Inlines(runs)) = stack.pop() {
                    push_block(&mut stack, Block::Paragraph(runs));
                }
            }
            Event::End(TagEnd::Heading(level)) => {
                if let Some(Frame::Inlines(runs)) = stack.pop() {
                    let level = match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        HeadingLevel::H4 => 4,
                        HeadingLevel::H5 => 5,
                        HeadingLevel::H6 => 6,
                    };
                    let anchor = slug(&plain(&runs));
                    push_block(&mut stack, Block::Heading { level, anchor, text: runs });
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info.split_whitespace().next().unwrap_or("").to_owned(),
                    CodeBlockKind::Indented => String::new(),
                };
                code = Some((lang, String::new()));
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((lang, text)) = code.take() {
                    close_loose(&mut stack);
                    push_block(&mut stack, Block::Code { lang, code: text.trim_end().to_owned(), source: None });
                }
            }
            Event::Start(Tag::List(start)) => {
                close_loose(&mut stack);
                stack.push(Frame::List { ordered: start.is_some(), items: Vec::new() });
            }
            Event::End(TagEnd::List(_)) => {
                if let Some(Frame::List { ordered, items }) = stack.pop() {
                    push_block(&mut stack, Block::List { ordered, items });
                }
            }
            Event::Start(Tag::Item) => stack.push(Frame::Item(Vec::new())),
            Event::End(TagEnd::Item) => {
                close_loose(&mut stack);
                if let Some(Frame::Item(blocks)) = stack.pop()
                    && let Some(Frame::List { items, .. }) = stack.last_mut()
                {
                    items.push(blocks);
                }
            }
            Event::Start(Tag::BlockQuote(_)) => stack.push(Frame::Blocks(Vec::new())),
            Event::End(TagEnd::BlockQuote(_)) => {
                if let Some(Frame::Blocks(blocks)) = stack.pop() {
                    push_block(&mut stack, Block::Quote(blocks));
                }
            }
            Event::Start(Tag::Table(_)) => {
                stack.push(Frame::Table { header: Vec::new(), rows: Vec::new(), row: Vec::new(), in_head: false })
            }
            Event::Start(Tag::TableHead) => {
                if let Some(Frame::Table { in_head, .. }) = stack.last_mut() {
                    *in_head = true;
                }
            }
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                if let Some(Frame::Table { header, rows, row, in_head }) = stack.last_mut() {
                    let cells = std::mem::take(row);
                    if *in_head {
                        *header = cells;
                        *in_head = false;
                    } else {
                        rows.push(cells);
                    }
                }
            }
            Event::Start(Tag::TableCell) => stack.push(Frame::Inlines(Vec::new())),
            Event::End(TagEnd::TableCell) => {
                if let Some(Frame::Inlines(runs)) = stack.pop()
                    && let Some(Frame::Table { row, .. }) = stack.last_mut()
                {
                    row.push(runs);
                }
            }
            Event::End(TagEnd::Table) => {
                if let Some(Frame::Table { header, rows, .. }) = stack.pop() {
                    push_block(&mut stack, Block::Table { header, rows });
                }
            }
            Event::Start(Tag::Strong) => style.strong += 1,
            Event::End(TagEnd::Strong) => style.strong = style.strong.saturating_sub(1),
            Event::Start(Tag::Emphasis) => style.emph += 1,
            Event::End(TagEnd::Emphasis) => style.emph = style.emph.saturating_sub(1),
            Event::Start(Tag::Link { dest_url, .. }) => style.link.push(classify(&dest_url, id, base)),
            Event::End(TagEnd::Link) => {
                style.link.pop();
            }
            Event::Text(t) => match &mut code {
                Some((_, buf)) => buf.push_str(&t),
                None => push_inline(&mut stack, &style, &t, false),
            },
            Event::Code(t) => push_inline(&mut stack, &style, &t, true),
            Event::SoftBreak => push_inline(&mut stack, &style, " ", false),
            Event::HardBreak => push_inline(&mut stack, &style, "\n", false),
            Event::Rule => push_block(&mut stack, Block::Rule),
            _ => {}
        }
    }
    match stack.into_iter().next() {
        Some(Frame::Blocks(blocks)) => blocks,
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx<'a>(repo: &'a Path, examples: &'a [Example]) -> PageContext<'a> {
        PageContext { repo, examples, components: &|| Ok("## Generated\n".into()) }
    }

    #[test]
    fn blocks_and_links() {
        let blocks = parse_blocks(
            "Read [theming](../concepts/theming.md#tokens), `nd check` and [src](../../../examples/hello/src/lib.rs).\n\n\
             - one\n- **two**\n\n| a | b |\n| - | - |\n| 1 | 2 |\n",
            "guides/x",
            "docs/content/guides",
        );
        let Block::Paragraph(runs) = &blocks[0] else { panic!("{blocks:?}") };
        assert_eq!(runs[1].link, Some(Link::Page { id: "concepts/theming".into(), anchor: Some("tokens".into()) }));
        assert!(runs[3].code);
        assert_eq!(runs[5].link, Some(Link::Repo("examples/hello/src/lib.rs".into())));
        let Block::List { items, .. } = &blocks[1] else { panic!() };
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[1][0], Block::Paragraph(r) if r[0].strong));
        assert!(matches!(&blocks[2], Block::Table { rows, .. } if rows.len() == 1));
    }

    #[test]
    fn directives_and_title() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("examples/hello/src")).unwrap();
        std::fs::write(dir.path().join("examples/hello/src/lib.rs"), "// docs:begin a\nfn a() {}\n// docs:end a\n")
            .unwrap();
        let examples = [Example { name: "hello".into(), title: "Hello".into(), ..Example::default() }];
        let text = "# Hello world\n\nIntro.\n\n<!-- docs:snippet examples/hello/src/lib.rs#a -->\n\n```md\n<!-- docs:example nope -->\n```\n\n<!-- docs:example hello -->\n";
        let p = page("start", text, &cx(dir.path(), &examples)).unwrap();
        assert_eq!(p.title, "Hello world");
        assert!(
            matches!(&p.blocks[1], Block::Code { code, source: Some(s), .. } if code == "fn a() {}" && s.line == 2)
        );
        assert!(matches!(&p.blocks[2], Block::Code { lang, .. } if lang == "md"));
        assert_eq!(p.blocks[3], Block::Example("hello".into()));
        assert!(p.markdown.contains("fn a() {}"));
        assert!(p.markdown.contains("examples/hello"));
        assert!(page("x", "<!-- docs:example nope -->\n", &cx(dir.path(), &examples)).is_err());
    }

    #[test]
    fn joins_paths() {
        assert_eq!(join("docs/content/a", "../../../examples/x").as_deref(), Some("examples/x"));
        assert_eq!(join("docs", "../../x"), None);
        assert_eq!(relative_to("docs/content/a", "examples/x"), "../../../examples/x");
    }
}
