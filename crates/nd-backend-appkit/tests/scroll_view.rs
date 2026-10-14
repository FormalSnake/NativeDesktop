//! Tier 2: a plain `overflow_y_scroll()` div on AppKit is an `NSScrollView` whose document holds
//! the children at their laid-out frames and is as tall as they are; moving the clip view shows
//! the rows further down.

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    harness::main("gate_m5::appkit_scroll_div", mac::run);
    #[cfg(not(target_os = "macos"))]
    harness::main("gate_m5::appkit_scroll_div", || {});
}

#[cfg(target_os = "macos")]
mod mac {
    use std::time::Duration;

    use nd_backend_appkit::AppHost;
    use nd_backend_appkit::objc2_foundation::NSPoint;
    use nd_ui::prelude::*;

    struct Scrolling;

    impl Render for Scrolling {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                v_flex()
                    .id("scroller")
                    .w(px(200.))
                    .h(px(300.))
                    .overflow_y_scroll()
                    .children((0..50).map(|i| div().id(format!("row-{i}")).w_full().h(px(40.)).flex_shrink_0())),
            )
        }
    }

    pub fn run() {
        let host = AppHost::new(|app| {
            app.open_window(WindowOptions::main("scroll").size(size(px(400.), px(400.))), |_, cx| {
                cx.new(|_| Scrolling)
            });
        });
        let pump = |host: &AppHost| {
            for _ in 0..10 {
                host.platform().pump(Duration::from_millis(1), Duration::from_millis(20));
                host.update(|app| {
                    if app.needs_frame() {
                        app.frame();
                    }
                });
            }
        };
        pump(&host);
        let views = host.backend().scroll_views();
        let (_, scroll) = views.first().expect("the div is an NSScrollView");
        let document = scroll.documentView().expect("document");
        let clip = scroll.contentView();
        let (doc_h, clip_h) = (document.frame().size.height, clip.bounds().size.height);
        assert!((doc_h - 2000.).abs() < 0.5, "document is as tall as its 50 rows: {doc_h}");
        assert!((clip_h - 300.).abs() < 0.5, "the clip is the div's height: {clip_h}");
        assert_eq!(document.subviews().len(), 50, "the rows are the document's children");
        clip.scrollToPoint(NSPoint::new(0., 1000.));
        scroll.reflectScrolledClipView(&clip);
        pump(&host);
        let visible = clip.documentVisibleRect();
        let row = document.subviews().objectAtIndex(25).frame();
        assert!(
            row.origin.y >= visible.origin.y
                && row.origin.y + row.size.height <= visible.origin.y + visible.size.height,
            "row 25 at {:?} is in the visible rect {:?}",
            row,
            visible
        );
        println!("ND_APPKIT_SCROLL_DIV_OK");
        std::process::exit(0);
    }
}
