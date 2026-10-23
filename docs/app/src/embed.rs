//! The four-tab example embed: Web runs the example live, macOS, Linux and Windows show the
//! captures `nd docs capture` took on that OS. A stale capture is never shown.

use nd::platform::{Feature, has};
use nd::prelude::*;
use nd_docs::captures::{Os, State};
use nd_docs::examples::Example;

use crate::blocks::open_url;
use crate::{Docs, repo_url};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Web,
    Os(Os),
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Web, Tab::Os(Os::Macos), Tab::Os(Os::Linux), Tab::Os(Os::Windows)];

    fn label(self) -> &'static str {
        match self {
            Tab::Web => "Web",
            Tab::Os(os) => os.label(),
        }
    }

    fn index(self) -> usize {
        Tab::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }
}

/// Where a capture file loads from: the package's assets/ on disk on the desktop, the bundle's
/// assets/ (relative to the page) on the web.
fn capture_source(example: &str, file: &str) -> ImageSource {
    if has(Feature::FileSystem) {
        format!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/captures/{}/{}"), example, file).into()
    } else {
        format!("assets/captures/{example}/{file}").into()
    }
}

impl Docs {
    fn tab(&self, example: &Example) -> Tab {
        self.tabs.get(&example.name).copied().unwrap_or(if example.web { Tab::Web } else { Tab::Os(Os::Macos) })
    }

    pub(crate) fn example(&self, name: &str, key: &str, theme: &Theme, cx: &Context<Self>) -> AnyElement {
        let Some(example) = self.content.site.example(name) else {
            return label(format!("No example named {name}")).secondary().into_any_element();
        };
        let tab = self.tab(example);
        let source = repo_url(&format!("examples/{}", example.name));
        let switch_name = example.name.clone();
        let header = h_flex()
            .w_full()
            .items_start()
            .gap(theme.spacing.md)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(theme.spacing.xs)
                    .child(
                        label(example.title.clone())
                            .text_size(theme.type_scale.headline)
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(label(example.summary.clone()).wraps().secondary()),
            )
            .child(
                link(format!("{key}/view-source"), "View source")
                    .keep_internal()
                    .on_click(move |_, _, _| open_url(&source)),
            );
        let tabs = segmented(format!("{key}/tabs"), Tab::ALL.map(Tab::label)).selected(tab.index()).on_change(
            cx.listener(move |this, e: &SelectChanged, _, cx| {
                if let Some(tab) = Tab::ALL.get(e.index) {
                    this.tabs.insert(switch_name.clone(), *tab);
                }
                cx.notify();
            }),
        );
        let body = match tab {
            Tab::Web => web_tab(example, key, theme),
            Tab::Os(os) => os_tab(example, os, self.content.capture(&example.name, os), theme),
        };
        v_flex()
            .w_full()
            .gap(theme.spacing.md)
            .p(theme.spacing.md)
            .rounded(theme.radius.lg)
            .border_1()
            .border_color(theme.border.subtle)
            .child(header)
            .child(tabs)
            .child(body)
            .into_any_element()
    }
}

fn web_tab(example: &Example, key: &str, theme: &Theme) -> AnyElement {
    match nd_docs_run::opener(&example.name).filter(|_| example.web) {
        Some(open) => h_flex()
            .gap(theme.spacing.md)
            .child(button(format!("{key}/run"), "Run live").on_click(move |_, _, cx| open(cx)))
            .child(label("Opens the example in its own window, running in this app.").wraps().secondary())
            .into_any_element(),
        None => v_flex()
            .w_full()
            .gap(theme.spacing.xs)
            .child(label("Runs on the desktop").font_weight(FontWeight::SEMIBOLD))
            .when_some(example.web_reason.clone(), |c, reason| c.child(label(reason).wraps().secondary()))
            .into_any_element(),
    }
}

fn os_tab(example: &Example, os: Os, state: &State, theme: &Theme) -> AnyElement {
    let State::Fresh(capture) = state else {
        return v_flex()
            .w_full()
            .items_center()
            .gap(theme.spacing.xs)
            .p(theme.spacing.xl)
            .rounded(theme.radius.md)
            .bg(theme.surface.raised)
            .child(label("Not captured yet").font_weight(FontWeight::SEMIBOLD))
            .child(label(format!("Run `nd docs capture` on {} to add it.", os.label())).wraps().secondary())
            .into_any_element();
    };
    let still = if theme.scheme.is_dark() { &capture.dark } else { &capture.light };
    let alt = format!("{} on {}", example.title, os.label());
    v_flex()
        .w_full()
        .gap(theme.spacing.sm)
        .child(
            image(capture_source(&example.name, still))
                .id(format!("{}/{}/still", example.name, os.key()))
                .alt(alt.clone())
                .contain()
                .w_full()
                .rounded(theme.radius.md),
        )
        .child(label(format!("Captured on {}, {}", capture.captured_on, capture.date)).secondary())
        .when_some(capture.recording.as_ref(), |c, recording| {
            c.child(
                image(capture_source(&example.name, recording))
                    .id(format!("{}/{}/recording", example.name, os.key()))
                    .alt(format!("{alt}, recording"))
                    .contain()
                    .w_full()
                    .rounded(theme.radius.md),
            )
        })
        .into_any_element()
}
