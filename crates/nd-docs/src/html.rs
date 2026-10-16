//! The static site: one HTML page per docs page, rendered from the same blocks the docs app
//! draws, so readers and crawlers without wasm (agents included) get every page. Page `a/b` is
//! written to `a/b.html` and linked as `/a/b`; `a/index` is linked as `/a/`. The docs app itself
//! sits under `/app/`, and capture images are loaded from its bundle.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::Result;

use crate::captures::{Manifest, Os, State};
use crate::examples::Example;
use crate::model::{Block, Inline, Link, Page, Site, SourceRef};

pub const SITE_URL: &str = "https://nativedesktop.formalsnake.dev";
pub const REPO_URL: &str = "https://github.com/FormalSnake/NativeDesktop";

/// The path a page is served at.
pub fn page_path(id: &str) -> String {
    match id {
        "index" => "/".to_owned(),
        _ => match id.strip_suffix("/index") {
            Some(dir) => format!("/{dir}/"),
            None => format!("/{id}"),
        },
    }
}

/// A repository path on GitHub (GitHub redirects `blob` to `tree` for a directory).
pub fn repo_url(path: &str) -> String {
    format!("{REPO_URL}/blob/main/{path}")
}

pub fn link_href(link: &Link) -> String {
    match link {
        Link::Page { id, anchor } => {
            let mut href = page_path(id);
            if let Some(a) = anchor {
                let _ = write!(href, "#{a}");
            }
            href
        }
        Link::Repo(path) => repo_url(path),
        Link::External(url) => url.clone(),
    }
}

fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

/// Writes every page, `404.html`, `robots.txt`, `sitemap.xml` and the `_headers` file for
/// Cloudflare's static assets.
pub fn write(site: &Site, captures: &Manifest, out: &Path) -> Result<()> {
    for page in &site.pages {
        let path = out.join(format!("{}.html", page.id));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, render_page(site, captures, page))?;
    }
    std::fs::write(out.join("404.html"), not_found(site))?;
    std::fs::write(out.join("robots.txt"), format!("User-agent: *\nAllow: /\n\nSitemap: {SITE_URL}/sitemap.xml\n"))?;
    let mut sitemap = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for page in &site.pages {
        let _ = writeln!(sitemap, "  <url><loc>{SITE_URL}{}</loc></url>", page_path(&page.id));
    }
    sitemap.push_str("</urlset>\n");
    std::fs::write(out.join("sitemap.xml"), sitemap)?;
    std::fs::write(out.join("_headers"), HEADERS)?;
    Ok(())
}

const HEADERS: &str = "\
/*.md
  Content-Type: text/markdown; charset=utf-8
/llms.txt
  Content-Type: text/plain; charset=utf-8
/llms-full.txt
  Content-Type: text/plain; charset=utf-8
";

fn render_page(site: &Site, captures: &Manifest, page: &Page) -> String {
    let mut body = String::new();
    let _ = writeln!(body, "<h1>{}</h1>", esc(&page.title));
    if !page.description.is_empty() {
        let _ = writeln!(body, "<p class=\"lede\">{}</p>", esc(&page.description));
    }
    for block in &page.blocks {
        render_block(&mut body, site, captures, block);
    }
    let _ = writeln!(
        body,
        "<footer><a href=\"/{id}.md\">Markdown source</a> · <a href=\"/llms.txt\">llms.txt</a> · \
         <a href=\"{edit}\">Edit on GitHub</a></footer>",
        id = page.id,
        edit = repo_url(&format!("docs/content/{}.md", page.id)),
    );
    let title = if page.id == "index" {
        "NativeDesktop docs".to_owned()
    } else {
        format!("{} | NativeDesktop docs", page.title)
    };
    shell(site, Some(page), &title, &page.description, &body)
}

fn not_found(site: &Site) -> String {
    let body = "<h1>Page not found</h1>\n<p>No docs page lives at this address. Start from the \
                <a href=\"/\">overview</a> or the <a href=\"/llms.txt\">page index</a>.</p>\n";
    shell(site, None, "Page not found | NativeDesktop docs", "", body)
}

fn shell(site: &Site, current: Option<&Page>, title: &str, description: &str, body: &str) -> String {
    let mut nav = String::new();
    for section in &site.sections {
        let _ = writeln!(nav, "<h2>{}</h2>\n<ul>", esc(&section.title));
        for id in &section.pages {
            let Some(page) = site.page(id) else { continue };
            let here = if current.is_some_and(|c| &c.id == id) { " aria-current=\"page\"" } else { "" };
            let _ = writeln!(nav, "<li><a href=\"{}\"{here}>{}</a></li>", page_path(id), esc(&page.title));
        }
        nav.push_str("</ul>\n");
    }
    let mut head = String::new();
    if !description.is_empty() {
        let _ = writeln!(head, "<meta name=\"description\" content=\"{}\">", esc(description));
    }
    if let Some(page) = current {
        let _ = writeln!(head, "<link rel=\"canonical\" href=\"{SITE_URL}{}\">", page_path(&page.id));
        let _ = writeln!(head, "<link rel=\"alternate\" type=\"text/markdown\" href=\"/{}.md\">", page.id);
    }
    format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
{head}<style>{CSS}</style>
</head>
<body>
<header class="top"><a class="brand" href="/">NativeDesktop</a><nav class="links"><a class="to-pages" href="#pages">Pages</a><a href="/app/">Docs app</a><a href="/llms.txt">llms.txt</a><a href="{REPO_URL}">GitHub</a></nav></header>
<div class="layout">
<main>
{body}</main>
<nav class="sidebar" id="pages" aria-label="Docs pages">
{nav}</nav>
</div>
</body>
</html>
"##,
        title = esc(title),
    )
}

fn inlines(out: &mut String, runs: &[Inline]) {
    for run in runs {
        let mut text = esc(&run.text).replace('\n', "<br>");
        if run.code {
            text = format!("<code>{text}</code>");
        }
        if run.emph {
            text = format!("<em>{text}</em>");
        }
        if run.strong {
            text = format!("<strong>{text}</strong>");
        }
        match &run.link {
            Some(link) => {
                let _ = write!(out, "<a href=\"{}\">{text}</a>", esc(&link_href(link)));
            }
            None => out.push_str(&text),
        }
    }
}

fn render_block(out: &mut String, site: &Site, captures: &Manifest, block: &Block) {
    match block {
        Block::Heading { level, anchor, text } => {
            let level = (*level).clamp(2, 6);
            let _ = write!(out, "<h{level} id=\"{anchor}\"><a class=\"anchor\" href=\"#{anchor}\">");
            inlines(out, text);
            let _ = writeln!(out, "</a></h{level}>");
        }
        Block::Paragraph(runs) => {
            out.push_str("<p>");
            inlines(out, runs);
            out.push_str("</p>\n");
        }
        Block::Code { lang, code, source } => {
            let class = if lang.is_empty() { String::new() } else { format!(" class=\"language-{}\"", esc(lang)) };
            let _ = writeln!(out, "<pre><code{class}>{}</code></pre>", esc(code));
            if let Some(SourceRef { path, line, .. }) = source {
                let _ = writeln!(
                    out,
                    "<p class=\"source\">From <a href=\"{}#L{line}\">{}</a>, line {line}.</p>",
                    repo_url(path),
                    esc(path)
                );
            }
        }
        Block::List { ordered, items } => {
            let tag = if *ordered { "ol" } else { "ul" };
            let _ = writeln!(out, "<{tag}>");
            for item in items {
                out.push_str("<li>");
                // A tight item is one paragraph: no <p> around it.
                match item.as_slice() {
                    [Block::Paragraph(runs)] => inlines(out, runs),
                    blocks => blocks.iter().for_each(|b| render_block(out, site, captures, b)),
                }
                out.push_str("</li>\n");
            }
            let _ = writeln!(out, "</{tag}>");
        }
        Block::Quote(blocks) => {
            out.push_str("<blockquote>\n");
            blocks.iter().for_each(|b| render_block(out, site, captures, b));
            out.push_str("</blockquote>\n");
        }
        Block::Table { header, rows } => {
            out.push_str("<div class=\"table\"><table>\n<thead><tr>");
            for cell in header {
                out.push_str("<th>");
                inlines(out, cell);
                out.push_str("</th>");
            }
            out.push_str("</tr></thead>\n<tbody>\n");
            for row in rows {
                out.push_str("<tr>");
                for cell in row {
                    out.push_str("<td>");
                    inlines(out, cell);
                    out.push_str("</td>");
                }
                out.push_str("</tr>\n");
            }
            out.push_str("</tbody></table></div>\n");
        }
        Block::Rule => out.push_str("<hr>\n"),
        Block::Example(name) => {
            if let Some(example) = site.example(name) {
                render_example(out, captures, example);
            }
        }
    }
}

fn render_example(out: &mut String, captures: &Manifest, example: &Example) {
    let _ = writeln!(
        out,
        "<figure class=\"example\">\n<figcaption><strong>Example: {}.</strong> {} Source in <a href=\"{}\">examples/{}</a>.",
        esc(&example.title),
        esc(&example.summary),
        repo_url(&format!("examples/{}", example.name)),
        esc(&example.name),
    );
    if example.web {
        out.push_str(" It also runs live in the <a href=\"/app/\">docs app</a>.");
    }
    out.push_str("</figcaption>\n");
    for os in Os::ALL {
        if let State::Fresh(c) = captures.state(example, os) {
            let dir = format!("/app/assets/captures/{}", example.name);
            let _ = writeln!(
                out,
                "<picture><source media=\"(prefers-color-scheme: dark)\" srcset=\"{dir}/{dark}\">\
                 <img src=\"{dir}/{light}\" alt=\"{title} on {os}\" loading=\"lazy\"></picture>\
                 <p class=\"source\">Captured on {on}, {date}.</p>",
                dark = esc(&c.dark),
                light = esc(&c.light),
                title = esc(&example.title),
                os = os.label(),
                on = esc(&c.captured_on),
                date = esc(&c.date),
            );
        }
    }
    out.push_str("</figure>\n");
}

/// Colours are `nd-theme`'s baked palette (`palette.rs`), light and dark.
const CSS: &str = r#"
:root{color-scheme:light dark;--surface:#fafafa;--sidebar:#f0f0f2;--raised:#ffffff;--text:#000000d9;--text2:#00000080;--accent:#2f6fed;--accent-container:#d9e5ff;--separator:#0000001a;--code:#0000000a}
@media (prefers-color-scheme:dark){:root{--surface:#1e1e20;--sidebar:#252528;--raised:#2b2b2e;--text:#ffffffd9;--text2:#ffffff8c;--accent:#4c8dff;--accent-container:#1f3a6e;--separator:#ffffff1f;--code:#ffffff0f}}
*{box-sizing:border-box}
html{-webkit-text-size-adjust:100%}
body{margin:0;background:var(--surface);color:var(--text);font:16px/1.6 -apple-system,BlinkMacSystemFont,"Segoe UI",system-ui,Ubuntu,sans-serif}
a{color:var(--accent);text-underline-offset:2px}
.top{display:flex;flex-wrap:wrap;align-items:center;justify-content:space-between;gap:16px;padding:12px 16px;border-bottom:1px solid var(--separator);background:var(--raised)}
.brand{font-weight:600;color:var(--text);text-decoration:none}
.links{display:flex;gap:16px;font-size:14px}
.layout{display:grid;grid-template-columns:260px minmax(0,1fr)}
.sidebar{grid-column:1;grid-row:1;background:var(--sidebar);border-right:1px solid var(--separator);padding:8px 16px 24px;font-size:14px;min-height:calc(100vh - 50px)}
main{grid-column:2;grid-row:1}
.to-pages{display:none}
.sidebar h2{font-size:12px;text-transform:uppercase;letter-spacing:.04em;color:var(--text2);margin:20px 0 6px}
.sidebar ul{list-style:none;margin:0;padding:0}
.sidebar li a{display:block;padding:3px 8px;border-radius:6px;color:var(--text);text-decoration:none}
.sidebar li a:hover{background:var(--separator)}
.sidebar a[aria-current]{background:var(--accent-container);font-weight:600}
main{padding:24px 40px 64px;max-width:820px;min-width:0}
h1{font-size:32px;line-height:1.2;margin:8px 0 8px}
h2{font-size:24px;margin:40px 0 8px}h3{font-size:19px;margin:28px 0 6px}h4,h5,h6{font-size:16px;margin:20px 0 4px}
h2,h3,h4,h5,h6{line-height:1.3;text-wrap:balance}
.anchor{color:inherit;text-decoration:none}
.lede{font-size:18px;color:var(--text2);margin-top:0}
code{font:13.5px/1.5 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;background:var(--code);padding:1px 4px;border-radius:4px}
pre{background:var(--raised);border:1px solid var(--separator);border-radius:8px;padding:12px 14px;overflow-x:auto}
pre code{background:none;padding:0}
.source{font-size:13px;color:var(--text2);margin-top:-8px}
blockquote{margin:16px 0;padding:4px 16px;border-left:3px solid var(--accent);background:var(--raised);border-radius:0 8px 8px 0}
.table{overflow-x:auto}
table{border-collapse:collapse;font-size:14px;margin:16px 0}
th,td{border:1px solid var(--separator);padding:6px 10px;text-align:left;vertical-align:top}
th{background:var(--sidebar)}
hr{border:0;border-top:1px solid var(--separator);margin:32px 0}
.example{margin:20px 0;padding:12px 16px;background:var(--raised);border:1px solid var(--separator);border-radius:8px}
.example img{max-width:100%;height:auto;display:block;margin-top:12px;border-radius:6px}
footer{margin-top:48px;padding-top:16px;border-top:1px solid var(--separator);font-size:13px;color:var(--text2)}
@media (max-width:760px){
.layout{grid-template-columns:minmax(0,1fr)}
.sidebar{grid-column:1;grid-row:2;min-height:0;border-right:0;border-top:1px solid var(--separator)}
main{grid-column:1;grid-row:1}
.to-pages{display:inline}
.links{gap:12px}
main{padding:16px 16px 48px}
h1{font-size:26px}
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_links() {
        assert_eq!(page_path("index"), "/");
        assert_eq!(page_path("agents/index"), "/agents/");
        assert_eq!(page_path("concepts/theming"), "/concepts/theming");
        let link = Link::Page { id: "concepts/theming".into(), anchor: Some("tokens".into()) };
        assert_eq!(link_href(&link), "/concepts/theming#tokens");
        assert_eq!(link_href(&Link::Repo("examples/hello".into())), format!("{REPO_URL}/blob/main/examples/hello"));
    }

    #[test]
    fn renders_blocks() {
        let page = Page {
            id: "a".into(),
            title: "A <b>".into(),
            blocks: vec![
                Block::Heading {
                    level: 2,
                    anchor: "x".into(),
                    text: vec![Inline { text: "X".into(), ..Default::default() }],
                },
                Block::Code { lang: "rust".into(), code: "a < b".into(), source: None },
            ],
            ..Default::default()
        };
        let site = Site { pages: vec![page.clone()], ..Default::default() };
        let html = render_page(&site, &Manifest::default(), &page);
        assert!(html.contains("<h1>A &lt;b&gt;</h1>"));
        assert!(html.contains("<h2 id=\"x\">"));
        assert!(html.contains("<code class=\"language-rust\">a &lt; b</code>"));
        assert!(html.contains("href=\"/a.md\""));
    }
}
