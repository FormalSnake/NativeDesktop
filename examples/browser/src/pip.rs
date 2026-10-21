//! `BROWSER_SCENE=pip`: the floating video chord, as nativebrowser runs it. A page plays a video
//! (a canvas stream, so no media file or network). Its own call to `requestPictureInPicture`
//! while loading is refused for want of a user gesture; the same call through
//! `execute_javascript` carries the app's gesture and lifts the video into Chromium's
//! picture-in-picture window, and a second run brings it back. Prints `ND_PIP_OK`, or
//! `ND_PIP_FAILED` and why, and exits.

use nd::prelude::*;
use nd::webview::{TitleChanged, WebView, webview};

const PAGE: &str = r#"<!doctype html><html><head><title>pip</title></head><body style="margin:0">
<canvas id="c" width="320" height="180" style="display:none"></canvas>
<video id="v" muted autoplay playsinline width="480" height="270"></video>
<script>
  const c = document.getElementById("c"), g = c.getContext("2d"), v = document.getElementById("v");
  let t = 0;
  setInterval(() => { g.fillStyle = `hsl(${t++ % 360},80%,50%)`; g.fillRect(0, 0, 320, 180); }, 33);
  v.srcObject = c.captureStream(30);
  v.addEventListener("playing", () => {
    v.requestPictureInPicture()
      .then(() => { document.title = "playing:allowed"; })
      .catch((e) => { document.title = "playing:" + e.name; });
  }, { once: true });
  v.addEventListener("enterpictureinpicture", (e) => {
    document.title = "pip:on:" + e.pictureInPictureWindow.width + "x" + e.pictureInPictureWindow.height;
  });
  v.addEventListener("leavepictureinpicture", () => { document.title = "pip:off"; });
</script></body></html>"#;

/// nativebrowser's `FLOAT_SCRIPT` (src/lib/float.ts): the playing video, else the largest with a
/// picture, same-origin frames included, into picture-in-picture, or back. Its answer lands in
/// the title, since a promise's value does not come back through `execute_javascript`.
const FLOAT: &str = r#"(async () => {
  const docs = [document];
  for (const f of document.querySelectorAll("iframe")) {
    try { if (f.contentDocument) docs.push(f.contentDocument); } catch (e) {}
  }
  for (const d of docs) {
    if (d.pictureInPictureElement) { await d.exitPictureInPicture(); return "off"; }
  }
  const videos = docs.flatMap((d) => [...d.querySelectorAll("video")]).filter((v) => v.readyState >= 1);
  const area = (v) => { const r = v.getBoundingClientRect(); return Math.max(0, r.width) * Math.max(0, r.height); };
  const playing = videos.filter((v) => !v.paused && !v.ended);
  const pool = playing.length ? playing : videos.filter((v) => v.videoWidth > 0);
  if (!pool.length) return "none";
  const video = pool.sort((a, b) => area(b) - area(a))[0];
  video.disablePictureInPicture = false;
  try {
    await video.requestPictureInPicture();
    return "on";
  } catch (e) {
    return "error:" + (e && e.message ? e.message : String(e));
  }
})().then((r) => { if (r !== "on" && r !== "off") document.title = "float:" + r; })"#;

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("ND_PIP_FAILED {why}");
    std::process::exit(1);
}

struct Scene {
    page: WebView,
    refused: Option<String>,
    window: Option<String>,
}

impl Scene {
    fn title(&mut self, title: &str) {
        if let Some(reason) = title.strip_prefix("float:") {
            fail(format!("the float script answered {reason}"));
        } else if let Some(answer) = title.strip_prefix("playing:") {
            if answer != "NotAllowedError" {
                fail(format!("the page's own request without a gesture was {answer}, not refused"));
            }
            eprintln!("ND_PIP refused without a gesture");
            self.refused = Some(answer.to_owned());
            self.page.execute_javascript(FLOAT, |_, _| {});
        } else if let Some(size) = title.strip_prefix("pip:on:") {
            eprintln!("ND_PIP floating {size}");
            self.window = Some(size.to_owned());
            self.page.execute_javascript(FLOAT, |_, _| {});
        } else if title == "pip:off" && self.window.is_some() {
            let (refused, window) = (self.refused.as_deref(), self.window.as_deref());
            eprintln!(
                "ND_PIP_OK without_gesture={} window={}",
                refused.unwrap_or_default(),
                window.unwrap_or_default()
            );
            std::process::exit(0);
        }
    }
}

impl Render for Scene {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        webview("web", &self.page)
            .size_full()
            .on_title_changed(cx.listener(|this, e: &TitleChanged, _, _| this.title(&e.title)))
    }
}

pub fn open(cx: &mut App) {
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(60));
        fail("no verdict within 60 s");
    });
    let page = WebView::new();
    page.load_html(PAGE, None);
    let options = WindowOptions::main("main").title("Floating video").size(size(px(720.), px(480.)));
    cx.open_window(options, move |_window, cx| cx.new(move |_cx| Scene { page, refused: None, window: None }));
}
