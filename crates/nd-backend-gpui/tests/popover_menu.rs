//! A popover with no set height, opened above a sliver anchor at the bottom of the window (a
//! chat composer's "+" menu), is drawn where it is laid out.

use std::sync::Arc;

use nd_backend_gpui::{GpuiBackend, GpuiChrome, GpuiHost};
use nd_components::Chrome;
use nd_gpui::gpui::HeadlessAppContext;
use nd_ui::backend::Backend;
use nd_ui::prelude::*;
use nd_ui::{Bezel, Placement, SemanticNode, WindowId};

struct Scene;

impl Render for Scene {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().size_full().justify_end().child(
            h_flex().h(px(50.)).items_end().child(
                div().id("anchor").w(px(1.)).h(px(30.)).child(
                    popover("menu")
                        .placement(Placement::Above)
                        .w(px(220.))
                        .p_1()
                        .child(button("files", "Files").bezel(Bezel::Borderless).w_full())
                        .child(button("gifs", "GIFs").bezel(Bezel::Borderless).w_full()),
                ),
            ),
        )
    }
}

fn find<'a>(node: &'a SemanticNode, test_id: &str) -> Option<&'a SemanticNode> {
    if node.test_id.as_deref() == Some(test_id) {
        return Some(node);
    }
    node.children.iter().find_map(|c| find(c, test_id))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let name = "popover_menu::a_popover_above_a_sliver_is_drawn";
    if args.iter().any(|a| a == "--list") {
        if !args.iter().any(|a| a == "--ignored") {
            println!("{name}: test");
        }
        return;
    }
    if args.iter().any(|a| a == "--ignored") {
        return;
    }
    if nd_gpui::gpui_platform::current_headless_renderer().ok().flatten().is_none() {
        println!("skipped: no headless renderer");
        return;
    }
    let text = nd_gpui::gpui_platform::current_platform(true).text_system();
    let mut cx =
        HeadlessAppContext::with_platform(text, Arc::new(()), nd_gpui::gpui_platform::current_headless_renderer);
    let mut window: Option<WindowId> = None;
    let host = cx.update(|cx| {
        GpuiHost::builder().quit_when_empty(false).start(cx, |app| {
            let backend = app.backend_as::<GpuiBackend>().expect("GPUI backend").clone();
            app.set_global(Chrome(GpuiChrome::new(backend)));
            let handle =
                app.open_window(WindowOptions::main("main").size(size(px(600.), px(400.))), |_, cx| cx.new(|_| Scene));
            window = Some(handle.id());
        })
    });
    let window = window.expect("window");
    for _ in 0..30 {
        cx.run_until_parked();
        cx.update(|cx| {
            host.pump(cx);
            host.draw(cx);
        });
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    let files = host.read(|app| find(&app.semantic_window(window).expect("window").root, "files").cloned());
    let bounds = files.and_then(|n| n.bounds).expect("the menu's first item is laid out");
    let paints = host.backend().paints(window).expect("paints");
    assert!(paints.iter().any(|p| p.label == "Files"), "the item's text was painted: {bounds:?}");
    let capture = host.job(|app| app.capture(window)).expect("capture");
    let image = image::load_from_memory(&capture.png).expect("png").to_rgba8();
    let scale = image.width() as f32 / 600.;
    let (x, y) = ((bounds.origin.x + 4.) * scale, (bounds.origin.y + bounds.size.height / 2.) * scale);
    let menu = image.get_pixel(x as u32, y as u32).0;
    let empty = image.get_pixel((590. * scale) as u32, (10. * scale) as u32).0;
    assert_ne!(menu, empty, "the menu shows where it is laid out ({x}, {y}): {menu:?} vs the window's {empty:?}");
    println!("test {name} ... ok");
}
