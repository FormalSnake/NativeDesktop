use gallery::{Page, Start};

fn page(page: Page) -> Start {
    Start { page, toast: page == Page::Toasts, ..Start::default() }
}

fn main() {
    let mut audit = nd::audit::Audit::new("gallery", gallery::app);
    for p in Page::ALL {
        audit = audit.window(p.id(), move |cx| gallery::open(cx, page(p)).id());
    }
    audit
        .window("palette", |cx| gallery::open(cx, Start { palette: Some("tri".into()), ..Start::default() }).id())
        .run()
}
