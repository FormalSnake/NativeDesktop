//! The docs pages load this bundle in an iframe at `/run/?example=<name>` when the reader asks to
//! run an example; it opens that example's window and nothing else.

fn main() -> nd::Result<()> {
    let name = example();
    nd_docs_run::app().run(move |cx| match nd_docs_run::opener(&name) {
        Some(open) => open(cx),
        None => eprintln!("nd-docs-run: no web example named `{name}`"),
    })
}

#[cfg(target_arch = "wasm32")]
fn example() -> String {
    let search = web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default();
    search
        .trim_start_matches('?')
        .split('&')
        .find_map(|pair| pair.strip_prefix("example="))
        .unwrap_or_default()
        .to_owned()
}

/// On the desktop the example comes from the first argument.
#[cfg(not(target_arch = "wasm32"))]
fn example() -> String {
    std::env::args().nth(1).unwrap_or_default()
}
