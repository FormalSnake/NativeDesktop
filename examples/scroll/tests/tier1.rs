//! Tier 1: the virtualized list on the test backend, natively and in a browser
//! (`#[nd::test]`). Only the rows in view exist as nodes.

use nd::test::{TestApp, tree};

#[nd::test]
fn a_hundred_thousand_rows_mount_only_what_shows(cx: &mut TestApp) {
    let window = scroll_bench::open(cx, 100_000, None).id();
    let root = tree(cx, window);
    let rows = root.iter().filter(|n| n.name.starts_with("Row ")).count();
    assert!(rows > 0 && rows < 100, "{rows} rows mounted");
    assert!(root.subtree_text().contains("Row 0"));
}
