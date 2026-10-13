//! Composed stand-ins for the `scaffold()` and `toolbar()` intents. `nd-components` replaces
//! them with the native mappings of ARCHITECTURE.md 3.6 (`NSSplitViewController`, `NSToolbar`);
//! the call sites stay the same.

use nd_ui::prelude::*;

pub struct Scaffold {
    toolbar: Option<Toolbar>,
    content: Option<AnyElement>,
}

pub fn scaffold() -> Scaffold {
    Scaffold { toolbar: None, content: None }
}

impl Scaffold {
    pub fn toolbar(mut self, toolbar: Toolbar) -> Self {
        self.toolbar = Some(toolbar);
        self
    }

    pub fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self
    }
}

impl IntoElement for Scaffold {
    fn into_any_element(self) -> AnyElement {
        v_flex()
            .size_full()
            .children(self.toolbar)
            .child(div().id("content").flex_1().min_h_0().children(self.content))
            .into_any_element()
    }
}

pub struct Toolbar {
    items: Vec<AnyElement>,
}

pub fn toolbar() -> Toolbar {
    Toolbar { items: Vec::new() }
}

impl Toolbar {
    pub fn item(mut self, item: impl IntoElement) -> Self {
        self.items.push(item.into_any_element());
        self
    }
}

impl IntoElement for Toolbar {
    fn into_any_element(self) -> AnyElement {
        h_flex().id("toolbar").gap_2().px_3().py_2().children(self.items).into_any_element()
    }
}
