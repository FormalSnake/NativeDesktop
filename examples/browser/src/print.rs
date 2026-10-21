//! `BROWSER_SCENE=print`: a two-page document saved as PDF without a dialog, then the OS print
//! dialog for it, as Cmd-P or Ctrl-P opens it in the browser. Prints `ND_PRINT pdf=...` once the
//! PDF checks out and `ND_PRINT_REQUESTED` once the dialog is asked for, and stays open: the gate
//! answers the dialog (the portal's on Linux) and ends the run. `ND_PRINT_FAILED` and why
//! otherwise.

use nd::prelude::*;
use nd::webview::{LoadingChanged, WebView, webview};

const PAGE: &str = r#"<!doctype html><html><head><title>Print me</title>
<style>section { break-after: page; font: 32px system-ui; }</style></head><body>
<section>First page</section><section>Second page</section></body></html>"#;

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("ND_PRINT_FAILED {why}");
    std::process::exit(1);
}

struct Scene {
    page: WebView,
    started: bool,
}

impl Scene {
    fn loaded(&mut self) {
        if std::mem::replace(&mut self.started, true) {
            return;
        }
        let path = std::env::temp_dir().join(format!("nd-print-{}.pdf", std::process::id()));
        let page = self.page.clone();
        self.page.print_to_pdf(path.clone(), move |result, _cx| {
            if let Err(e) = result {
                fail(format!("print_to_pdf: {e}"));
            }
            let bytes = std::fs::read(&path).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
            let _ = std::fs::remove_file(&path);
            if !bytes.starts_with(b"%PDF-") {
                fail("print_to_pdf wrote something other than a PDF");
            }
            let pages = bytes.windows(12).filter(|w| w.starts_with(b"/Type /Page") && w[11] != b's').count();
            eprintln!("ND_PRINT pdf={} bytes pages={pages}", bytes.len());
            page.print();
            eprintln!("ND_PRINT_REQUESTED");
        });
    }
}

impl Render for Scene {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        webview("web", &self.page).size_full().on_loading_changed(cx.listener(|this, e: &LoadingChanged, _, _| {
            if !e.loading {
                this.loaded();
            }
        }))
    }
}

pub fn open(cx: &mut App) {
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(90));
        fail("the gate never ended the run");
    });
    let page = WebView::new();
    page.load_html(PAGE, None);
    let options = WindowOptions::main("main").title("Print").size(size(px(720.), px(480.)));
    cx.open_window(options, move |_window, cx| cx.new(move |_cx| Scene { page, started: false }));
}
