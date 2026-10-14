//! The app's items in a web view's context menu, and which of them one click earns.
//!
//! In [`ContextMenuMode::Native`] the engine's own menu opens (Back, Reload, Copy Image, spelling,
//! Inspect Element) and the items that survive the click's hit test are appended after a
//! separator. In [`ContextMenuMode::Suppress`] no menu opens and the app builds its own from the
//! [`crate::ContextMenu`] event, which fires in both modes.

use bitflags::bitflags;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContextMenuMode {
    #[default]
    Native,
    Suppress,
}

bitflags! {
    /// What a click can land on. `PAGE` means nothing more specific: a link, image, selection
    /// or editable field wins over it, as in a browser's own menu.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct Contexts: u8 {
        const PAGE = 1;
        const LINK = 1 << 1;
        const IMAGE = 1 << 2;
        const SELECTION = 1 << 3;
        const EDITABLE = 1 << 4;
        const ALL = Self::PAGE.bits() | Self::LINK.bits() | Self::IMAGE.bits() | Self::SELECTION.bits() | Self::EDITABLE.bits();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MenuItemKind {
    #[default]
    Normal,
    Checkbox,
    /// Contiguous radio siblings form one group.
    Radio,
    Separator,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem {
    /// Echoed back in [`crate::ContextMenuItemClicked`]; empty for a separator.
    pub id: String,
    pub label: String,
    pub kind: MenuItemKind,
    pub checked: bool,
    pub enabled: bool,
    pub contexts: Contexts,
    /// `*` globs matched against the link or image URL under the pointer, whichever context
    /// matched. Empty matches everything.
    pub target_url_globs: Vec<String>,
    /// A submenu, dropped when every child is filtered out.
    pub children: Vec<MenuItem>,
}

impl MenuItem {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind: MenuItemKind::Normal,
            checked: false,
            enabled: true,
            contexts: Contexts::PAGE,
            target_url_globs: Vec::new(),
            children: Vec::new(),
        }
    }

    pub fn separator() -> Self {
        Self { kind: MenuItemKind::Separator, contexts: Contexts::ALL, ..Self::new("", "") }
    }

    pub fn checkbox(id: impl Into<String>, label: impl Into<String>, checked: bool) -> Self {
        Self { kind: MenuItemKind::Checkbox, checked, ..Self::new(id, label) }
    }

    pub fn radio(id: impl Into<String>, label: impl Into<String>, checked: bool) -> Self {
        Self { kind: MenuItemKind::Radio, checked, ..Self::new(id, label) }
    }

    pub fn submenu(label: impl Into<String>, children: Vec<MenuItem>) -> Self {
        Self { children, ..Self::new("", label) }
    }

    pub fn contexts(mut self, contexts: Contexts) -> Self {
        self.contexts = contexts;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn target_url_globs(mut self, globs: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.target_url_globs = globs.into_iter().map(Into::into).collect();
        self
    }

    fn matches(&self, hit: &Hit) -> bool {
        if self.kind == MenuItemKind::Separator {
            return true;
        }
        let bits = hit.contexts() & self.contexts;
        if bits.is_empty() {
            return false;
        }
        if self.target_url_globs.is_empty() {
            return true;
        }
        let any = |url: &Option<String>| {
            url.as_deref().is_some_and(|u| !u.is_empty() && self.target_url_globs.iter().any(|g| glob(g, u)))
        };
        (bits.contains(Contexts::LINK) && any(&hit.link)) || (bits.contains(Contexts::IMAGE) && any(&hit.image))
    }

    /// Matches the hit and, for a submenu, keeps at least one non-separator child.
    pub fn survives(&self, hit: &Hit) -> bool {
        self.matches(hit)
            && (self.children.is_empty()
                || self.children.iter().any(|c| c.kind != MenuItemKind::Separator && c.survives(hit)))
    }
}

/// What the click under a context menu landed on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hit {
    pub link: Option<String>,
    pub image: Option<String>,
    pub selection: Option<String>,
    pub editable: bool,
}

impl Hit {
    pub fn contexts(&self) -> Contexts {
        let mut bits = Contexts::empty();
        let set = |s: &Option<String>| s.as_deref().is_some_and(|s| !s.is_empty());
        if set(&self.link) {
            bits |= Contexts::LINK;
        }
        if set(&self.image) {
            bits |= Contexts::IMAGE;
        }
        if set(&self.selection) {
            bits |= Contexts::SELECTION;
        }
        if self.editable {
            bits |= Contexts::EDITABLE;
        }
        if bits.is_empty() { Contexts::PAGE } else { bits }
    }
}

/// One entry of the menu a click earns, with separators only where something survived on both
/// sides.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry<'a> {
    Item(&'a MenuItem),
    Submenu(&'a MenuItem, Vec<Entry<'a>>),
    Separator,
}

/// The items `hit` earns, in order. Empty when nothing survives, so the engine's menu gets no
/// trailing separator.
pub fn filter<'a>(items: &'a [MenuItem], hit: &Hit) -> Vec<Entry<'a>> {
    let mut out = Vec::new();
    let mut pending_separator = false;
    for item in items {
        if item.kind == MenuItemKind::Separator {
            pending_separator = !out.is_empty();
            continue;
        }
        if !item.survives(hit) {
            continue;
        }
        if pending_separator {
            out.push(Entry::Separator);
            pending_separator = false;
        }
        if item.children.is_empty() {
            out.push(Entry::Item(item));
        } else {
            out.push(Entry::Submenu(item, filter(&item.children, hit)));
        }
    }
    out
}

/// The `checked` and `was_checked` a click on `item` reports.
pub fn implied_check(item: &MenuItem) -> (Option<bool>, Option<bool>) {
    match item.kind {
        MenuItemKind::Checkbox => (Some(!item.checked), Some(item.checked)),
        MenuItemKind::Radio => (Some(true), Some(item.checked)),
        _ => (None, None),
    }
}

/// `*` matches any run of characters, everything else itself.
pub fn glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti, mut star, mut star_t) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            star_t = ti;
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            star_t += 1;
            ti = star_t;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(url: &str) -> Hit {
        Hit { link: Some(url.into()), ..Hit::default() }
    }

    #[test]
    fn page_means_nothing_more_specific() {
        assert_eq!(Hit::default().contexts(), Contexts::PAGE);
        assert_eq!(link("https://a").contexts(), Contexts::LINK);
        let page = MenuItem::new("p", "Page");
        assert!(page.survives(&Hit::default()));
        assert!(!page.survives(&link("https://a")));
    }

    #[test]
    fn globs_test_the_url_of_the_matched_context() {
        let item = MenuItem::new("l", "Link").contexts(Contexts::LINK).target_url_globs(["*://*.example.com/*"]);
        assert!(item.survives(&link("https://www.example.com/x")));
        assert!(!item.survives(&link("https://other.org/")));
        let image = Hit { image: Some("https://www.example.com/i.png".into()), ..Hit::default() };
        assert!(!item.survives(&image), "a link item never tests the image URL");
    }

    #[test]
    fn separators_only_between_survivors_and_empty_submenus_drop() {
        let items = vec![
            MenuItem::separator(),
            MenuItem::new("a", "A"),
            MenuItem::separator(),
            MenuItem::new("l", "L").contexts(Contexts::LINK),
            MenuItem::separator(),
            MenuItem::submenu("Sub", vec![MenuItem::new("s", "S").contexts(Contexts::IMAGE)]),
            MenuItem::separator(),
            MenuItem::new("b", "B"),
        ];
        let out = filter(&items, &Hit::default());
        let ids: Vec<&str> = out
            .iter()
            .map(|e| match e {
                Entry::Item(i) => i.id.as_str(),
                Entry::Separator => "|",
                Entry::Submenu(..) => "sub",
            })
            .collect();
        assert_eq!(ids, ["a", "|", "b"]);
    }

    #[test]
    fn checkbox_and_radio_report_the_implied_state() {
        assert_eq!(implied_check(&MenuItem::checkbox("c", "C", true)), (Some(false), Some(true)));
        assert_eq!(implied_check(&MenuItem::radio("r", "R", false)), (Some(true), Some(false)));
        assert_eq!(implied_check(&MenuItem::new("n", "N")), (None, None));
    }

    #[test]
    fn glob_matches_stars_anywhere() {
        assert!(glob("*", ""));
        assert!(glob("a*c", "abbbc"));
        assert!(glob("*.png", "x.png"));
        assert!(!glob("a*c", "abd"));
    }
}
