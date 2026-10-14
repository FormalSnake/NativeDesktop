//! Chromium's context menu off-screen: it only hands over the model, so showing it (natively on
//! macOS, drawn on Linux) and merging the app's items in are ours.

use cef::*;
use nd_webview::menu::Hit;

/// What the user picked.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Choice {
    Chromium(i32),
    App(nd_webview::MenuItem),
}

/// A menu label without Chromium's `&` mnemonic marks, keeping escaped `&&` as `&`.
pub(crate) fn label(text: &str) -> String {
    text.replace("&&", "\u{0}").replace('&', "").replace('\u{0}', "&")
}

/// What a context menu hit, from Chromium's parameters.
pub(crate) fn hit(params: &ContextMenuParams) -> Hit {
    let some = |s: String| Some(s).filter(|s| !s.is_empty());
    let image = (params.media_type() == ContextMenuMediaType::IMAGE).then(|| crate::text(&params.source_url()));
    Hit {
        link: some(crate::text(&params.link_url())),
        image: image.and_then(some),
        selection: some(crate::text(&params.selection_text())),
        editable: params.is_editable() != 0,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn mnemonics_are_dropped_and_escaped_ampersands_kept() {
        assert_eq!(super::label("&Back"), "Back");
        assert_eq!(super::label("Save && Close"), "Save & Close");
    }
}
