//! Tier 2: the appearance audit (`nd::audit`) over the docs, light, dark and matugen: the first
//! page, and the first page with each kind of block that draws its own surface.

use nd_docs::Block;
use nd_docs_app::{Content, Start};

fn first_with(content: &Content, wanted: fn(&Block) -> bool) -> Option<String> {
    content.site.pages.iter().find(|p| p.blocks.iter().any(wanted)).map(|p| p.id.clone())
}

fn main() {
    let content = Content::load().unwrap_or_else(|e| panic!("the docs content: {e}"));
    let mut pages: Vec<String> = content.site.pages.first().map(|p| p.id.clone()).into_iter().collect();
    let kinds: [fn(&Block) -> bool; 4] = [
        |b| matches!(b, Block::Example(_)),
        |b| matches!(b, Block::Code { .. }),
        |b| matches!(b, Block::Table { .. }),
        |b| matches!(b, Block::Quote(_)),
    ];
    for kind in kinds {
        if let Some(id) = first_with(&content, kind).filter(|id| !pages.contains(id)) {
            pages.push(id);
        }
    }
    let mut audit = nd::audit::Audit::new("nd-docs-app", nd_docs_app::app);
    for id in pages {
        let label = id.replace('/', "-");
        audit = audit
            .window(&label, move |cx| nd_docs_app::open(cx, Start { page: Some(id.clone()), ..Start::default() }).id());
    }
    audit.run()
}
