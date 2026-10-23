//! Tier 1: the docs on the test backend, natively and in a browser (`#[nd::test]`).

use nd::prelude::*;
use nd::test::{TestApp, by_test_id, text};
use nd_docs_app::{Content, Start};

#[nd::test]
fn every_page_opens_from_the_sidebar(cx: &mut TestApp) {
    let content = Content::load().unwrap_or_else(|e| panic!("the docs content: {e}"));
    assert!(!content.site.pages.is_empty());
    let window = nd_docs_app::open(cx, Start::default());
    for page in &content.site.pages {
        let tree = by_test_id(cx, window.id(), "pages").unwrap_or_else(|| panic!("no source tree"));
        cx.emit(tree.id, SourceSelected { id: page.id.clone().into() });
        assert_eq!(window.root().read(cx).page(), page.id);
        let shown = text(cx, window.id());
        assert!(shown.contains(&page.title), "{}: no title in {shown}", page.id);
    }
}

/// The embeds load: every example the Web tab runs live has an opener, and it opens a window.
#[nd::test]
fn every_web_example_has_an_opener(cx: &mut TestApp) {
    let content = Content::load().unwrap_or_else(|e| panic!("the docs content: {e}"));
    for example in content.site.examples.iter().filter(|e| e.web) {
        let open = nd_docs_run::opener(&example.name)
            .unwrap_or_else(|| panic!("{} has web = true but no opener in docs/run/src/lib.rs", example.name));
        let before = cx.window_ids().count();
        open(cx);
        cx.run_until_idle();
        assert!(cx.window_ids().count() > before, "{} opened no window", example.name);
    }
}
