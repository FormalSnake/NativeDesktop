//! Chromium's context menu as an `NSMenu`, with the app's items appended. Off-screen, Chromium
//! only hands over the model; showing it, and the system look, are ours.

use std::cell::RefCell;
use std::rc::Rc;

use cef::*;
use nd_webview::MenuItemKind;
use nd_webview::menu::{Entry, Hit};
use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSControlStateValueOff, NSControlStateValueOn, NSMenu, NSMenuItem};
use objc2_foundation::{MainThreadMarker, NSObject, NSString};

/// What the user picked.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Choice {
    Chromium(i32),
    App(nd_webview::MenuItem),
}

pub(crate) struct Built {
    pub menu: Retained<NSMenu>,
    pub chosen: Rc<RefCell<Option<Choice>>>,
    _target: Retained<Target>,
}

fn label(text: &str) -> Retained<NSString> {
    // Chromium marks mnemonics with `&`, which AppKit shows literally.
    NSString::from_str(&text.replace("&&", "\u{0}").replace('&', "").replace('\u{0}', "&"))
}

pub(crate) fn build(mtm: MainThreadMarker, model: Option<&MenuModel>, app: &[Entry<'_>]) -> Built {
    let chosen = Rc::new(RefCell::new(None));
    let target = Target::new(mtm, chosen.clone());
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    if let Some(model) = model {
        add_model(mtm, &menu, model, &target);
    }
    if !app.is_empty() {
        if menu.numberOfItems() > 0 {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
        }
        add_app(mtm, &menu, app, &target);
    }
    Built { menu, chosen, _target: target }
}

fn add_model(mtm: MainThreadMarker, menu: &NSMenu, model: &MenuModel, target: &Target) {
    for i in 0..model.count() {
        if model.is_visible_at(i) == 0 {
            continue;
        }
        let kind = model.type_at(i);
        if kind == MenuItemType::SEPARATOR {
            if menu.numberOfItems() > 0 {
                menu.addItem(&NSMenuItem::separatorItem(mtm));
            }
            continue;
        }
        let title = label(&super::text(&model.label_at(i)));
        if kind == MenuItemType::SUBMENU {
            let Some(sub_model) = model.sub_menu_at(i) else { continue };
            let sub = NSMenu::initWithTitle(NSMenu::alloc(mtm), &title);
            sub.setAutoenablesItems(false);
            add_model(mtm, &sub, &sub_model, target);
            let parent = NSMenuItem::new(mtm);
            parent.setTitle(&title);
            parent.setSubmenu(Some(&sub));
            menu.addItem(&parent);
            continue;
        }
        let item = item(mtm, &title, sel!(chromium:), target);
        item.setTag(model.command_id_at(i) as isize);
        item.setEnabled(model.is_enabled_at(i) != 0);
        if model.is_checked_at(i) != 0 {
            item.setState(NSControlStateValueOn);
        }
        menu.addItem(&item);
    }
}

fn add_app(mtm: MainThreadMarker, menu: &NSMenu, entries: &[Entry<'_>], target: &Target) {
    for entry in entries {
        match entry {
            Entry::Separator => menu.addItem(&NSMenuItem::separatorItem(mtm)),
            Entry::Submenu(item, children) => {
                let title = NSString::from_str(&item.label);
                let sub = NSMenu::initWithTitle(NSMenu::alloc(mtm), &title);
                sub.setAutoenablesItems(false);
                add_app(mtm, &sub, children, target);
                let parent = NSMenuItem::new(mtm);
                parent.setTitle(&title);
                parent.setEnabled(item.enabled);
                parent.setSubmenu(Some(&sub));
                menu.addItem(&parent);
            }
            Entry::Item(app_item) => {
                let entry = item(mtm, &NSString::from_str(&app_item.label), sel!(app:), target);
                entry.setEnabled(app_item.enabled);
                if matches!(app_item.kind, MenuItemKind::Checkbox | MenuItemKind::Radio) {
                    entry.setState(if app_item.checked { NSControlStateValueOn } else { NSControlStateValueOff });
                }
                let mut items = target.ivars().items.borrow_mut();
                entry.setTag(items.len() as isize);
                items.push((*app_item).clone());
                menu.addItem(&entry);
            }
        }
    }
}

fn item(mtm: MainThreadMarker, title: &NSString, action: objc2::runtime::Sel, target: &Target) -> Retained<NSMenuItem> {
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), title, Some(action), &NSString::new())
    };
    unsafe { item.setTarget(Some(target)) };
    item
}

/// What a context menu hit, from Chromium's parameters.
pub(crate) fn hit(params: &ContextMenuParams) -> Hit {
    let some = |s: String| Some(s).filter(|s| !s.is_empty());
    let image = (params.media_type() == ContextMenuMediaType::IMAGE).then(|| super::text(&params.source_url()));
    Hit {
        link: some(super::text(&params.link_url())),
        image: image.and_then(some),
        selection: some(super::text(&params.selection_text())),
        editable: params.is_editable() != 0,
    }
}

pub struct TargetIvars {
    chosen: Rc<RefCell<Option<Choice>>>,
    items: RefCell<Vec<nd_webview::MenuItem>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements. No Drop impl.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = TargetIvars]
    pub struct Target;

    unsafe impl NSObjectProtocol for Target {}

    impl Target {
        #[unsafe(method(chromium:))]
        fn chromium(&self, sender: &NSMenuItem) {
            *self.ivars().chosen.borrow_mut() = Some(Choice::Chromium(sender.tag() as i32));
        }

        #[unsafe(method(app:))]
        fn app(&self, sender: &NSMenuItem) {
            let item = self.ivars().items.borrow().get(sender.tag() as usize).cloned();
            *self.ivars().chosen.borrow_mut() = item.map(Choice::App);
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker, chosen: Rc<RefCell<Option<Choice>>>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars { chosen, items: RefCell::default() });
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn mnemonics_are_dropped_and_escaped_ampersands_kept() {
        assert_eq!(super::label("&Back").to_string(), "Back");
        assert_eq!(super::label("Save && Close").to_string(), "Save & Close");
    }
}
