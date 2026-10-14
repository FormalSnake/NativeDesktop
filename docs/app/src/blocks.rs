//! A page's blocks as elements. nd has no rich text yet, so a paragraph is one label of its runs
//! and its links follow it as a row of link controls.

use nd::prelude::*;
use nd_docs::model::plain;
use nd_docs::{Block, Inline, Link as DocLink};

use crate::{Docs, repo_url};

impl Docs {
    /// `key` is unique per block on the page and prefixes the ids of the controls inside it.
    pub(crate) fn block(&self, block: &Block, key: &str, theme: &Theme, cx: &Context<Self>) -> AnyElement {
        match block {
            Block::Heading { level, anchor, text } => {
                let (size, weight) = match level {
                    1 => (theme.type_scale.title, FontWeight::BOLD),
                    2 => (theme.type_scale.title, FontWeight::SEMIBOLD),
                    3 => (theme.type_scale.headline, FontWeight::SEMIBOLD),
                    _ => (theme.type_scale.body, FontWeight::SEMIBOLD),
                };
                div()
                    .pt(theme.spacing.lg)
                    .child(label(plain(text)).id(format!("h:{anchor}")).wraps().text_size(size).font_weight(weight))
                    .into_any_element()
            }
            Block::Paragraph(runs) => self.paragraph(runs, key, theme, cx),
            Block::Code { code, source, .. } => {
                let well = div()
                    .w_full()
                    .p(theme.spacing.md)
                    .rounded(theme.radius.md)
                    .bg(theme.surface.raised)
                    .border_1()
                    .border_color(theme.border.subtle)
                    .child(
                        label(code.trim_end().to_owned())
                            .id(format!("{key}/code"))
                            .wraps()
                            .selectable()
                            .font(theme.fonts.mono.clone())
                            .text_size(theme.type_scale.footnote),
                    );
                v_flex()
                    .w_full()
                    .gap(theme.spacing.xs)
                    .child(well)
                    .when_some(source.as_ref(), |c, s| {
                        let url = format!("{}#L{}", repo_url(&s.path), s.line);
                        c.child(
                            link(format!("{key}/source"), format!("From {}, line {}", s.path, s.line))
                                .keep_internal()
                                .on_click(move |_, _, _| open_url(&url)),
                        )
                    })
                    .into_any_element()
            }
            Block::List { ordered, items } => {
                let rows: Vec<AnyElement> = items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| {
                        let marker = if *ordered { format!("{}.", i + 1) } else { "\u{2022}".to_owned() };
                        let inner: Vec<AnyElement> = item
                            .iter()
                            .enumerate()
                            .map(|(j, b)| self.block(b, &format!("{key}/{i}/{j}"), theme, cx))
                            .collect();
                        h_flex()
                            .w_full()
                            .items_start()
                            .gap(theme.spacing.sm)
                            .child(div().min_w(theme.spacing.lg).child(label(marker).secondary()))
                            .child(v_flex().flex_1().min_w_0().gap(theme.spacing.xs).children(inner))
                            .into_any_element()
                    })
                    .collect();
                v_flex().w_full().gap(theme.spacing.xs).children(rows).into_any_element()
            }
            Block::Quote(blocks) => {
                let inner: Vec<AnyElement> =
                    blocks.iter().enumerate().map(|(i, b)| self.block(b, &format!("{key}/{i}"), theme, cx)).collect();
                v_flex()
                    .w_full()
                    .p(theme.spacing.md)
                    .gap(theme.spacing.sm)
                    .rounded(theme.radius.md)
                    .bg(theme.surface.raised)
                    .children(inner)
                    .into_any_element()
            }
            Block::Table { header, rows } => {
                let row = |cells: &[Vec<Inline>], strong: bool| {
                    h_flex().w_full().items_start().children(cells.iter().map(|cell| {
                        div().flex_1().min_w_0().px(theme.spacing.sm).py(theme.spacing.xs).child(
                            label(plain(cell))
                                .wraps()
                                .selectable()
                                .when(strong, |l| l.font_weight(FontWeight::SEMIBOLD)),
                        )
                    }))
                };
                let mut table = v_flex()
                    .w_full()
                    .rounded(theme.radius.md)
                    .border_1()
                    .border_color(theme.border.subtle)
                    .child(row(header, true).bg(theme.surface.raised));
                for cells in rows {
                    table = table.child(separator()).child(row(cells, false));
                }
                table.into_any_element()
            }
            Block::Rule => separator().into_any_element(),
            Block::Example(name) => self.example(name, key, theme, cx),
        }
    }

    fn paragraph(&self, runs: &[Inline], key: &str, theme: &Theme, cx: &Context<Self>) -> AnyElement {
        let mut links: Vec<AnyElement> = Vec::new();
        for (i, run) in runs.iter().enumerate() {
            let Some(target) = &run.link else { continue };
            let id = format!("{key}/link{i}");
            let control = match target.clone() {
                DocLink::Page { id: page, .. } => link(id, run.text.clone()).keep_internal().on_click(cx.listener(
                    move |this, _: &LinkActivated, _, cx| {
                        this.show(&page);
                        cx.notify();
                    },
                )),
                DocLink::Repo(path) => {
                    let url = repo_url(&path);
                    link(id, run.text.clone()).keep_internal().on_click(move |_, _, _| open_url(&url))
                }
                DocLink::External(url) => {
                    link(id, run.text.clone()).keep_internal().on_click(move |_, _, _| open_url(&url))
                }
            };
            links.push(control.into_any_element());
        }
        v_flex()
            .w_full()
            .gap(theme.spacing.xs)
            .child(label(plain(runs)).id(format!("{key}/text")).wraps().selectable())
            .when(!links.is_empty(), |c| c.child(h_flex().flex_wrap().gap(theme.spacing.md).children(links)))
            .into_any_element()
    }
}

/// Opens `url` in the browser. A failure leaves the page as it is; there is nothing to retry.
pub(crate) fn open_url(url: &str) {
    let _ = nd::system::open_url(url);
}
