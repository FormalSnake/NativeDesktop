//! The app's items in WebKit's own context menu.

use std::time::Instant;

use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSControlStateValueOff, NSControlStateValueOn, NSMenu, NSMenuItem, NSUserInterfaceItemIdentification,
};
use objc2_foundation::{MainThreadMarker, NSObject, NSString};

use super::WkPage;
use crate::events::ContextMenuItemClicked;
use crate::menu::{Entry, Hit, MenuItem, MenuItemKind, filter, implied_check};

/// What a click on one appended item reports.
pub(crate) struct Action {
    item: MenuItem,
    hit: Hit,
}

/// WebKit's stock item identifiers say what was clicked when the agent's report is missing.
fn stock_hit(menu: &NSMenu, last_hover: &str) -> Hit {
    let mut hit = Hit::default();
    for item in menu.itemArray().iter() {
        let id = item.identifier().map(|i| i.to_string()).unwrap_or_default();
        if id.contains("Link") && !last_hover.is_empty() {
            hit.link = Some(last_hover.to_owned());
        }
        if id.contains("Image") {
            hit.image = Some("about:image".into());
        }
        if id.contains("Copy") && !id.contains("Link") && !id.contains("Image") {
            hit.selection = Some(String::new());
        }
        if id.contains("Paste") || id.contains("Cut") {
            hit.editable = true;
        }
    }
    hit
}

pub(super) fn append(owner: &WkPage, menu: &NSMenu) {
    let mtm = owner.web.mtm();
    let hit = {
        let st = owner.st.borrow();
        match &st.last_hit {
            Some((at, hit)) if at.elapsed().as_secs_f64() < super::HIT_FRESHNESS => hit.clone(),
            _ => stock_hit(menu, &st.last_hover),
        }
    };
    let items = owner.st.borrow().menu_items.clone();
    let entries = filter(&items, &hit);
    owner.st.borrow_mut().menu_actions.clear();
    if entries.is_empty() {
        return;
    }
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    let target = Target::new(mtm, owner.this.clone());
    add(owner, mtm, menu, &entries, &hit, &target);
    // NSMenuItem holds its target weakly; the menu keeps it alive through this item.
    let keeper = NSMenuItem::new(mtm);
    keeper.setHidden(true);
    unsafe { keeper.setRepresentedObject(Some(&target)) };
    menu.addItem(&keeper);
}

fn add(owner: &WkPage, mtm: MainThreadMarker, menu: &NSMenu, entries: &[Entry<'_>], hit: &Hit, target: &Target) {
    for entry in entries {
        match entry {
            Entry::Separator => menu.addItem(&NSMenuItem::separatorItem(mtm)),
            Entry::Submenu(item, children) => {
                let title = NSString::from_str(&item.label);
                let parent = NSMenuItem::new(mtm);
                parent.setTitle(&title);
                parent.setEnabled(item.enabled);
                let sub = NSMenu::initWithTitle(NSMenu::alloc(mtm), &title);
                sub.setAutoenablesItems(false);
                add(owner, mtm, &sub, children, hit, target);
                parent.setSubmenu(Some(&sub));
                menu.addItem(&parent);
            }
            Entry::Item(item) => {
                let entry = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str(&item.label),
                        Some(sel!(run:)),
                        &NSString::new(),
                    )
                };
                entry.setEnabled(item.enabled);
                if matches!(item.kind, MenuItemKind::Checkbox | MenuItemKind::Radio) {
                    entry.setState(if item.checked { NSControlStateValueOn } else { NSControlStateValueOff });
                }
                let mut st = owner.st.borrow_mut();
                entry.setTag(st.menu_actions.len() as isize);
                st.menu_actions.push(Action { item: (*item).clone(), hit: hit.clone() });
                unsafe { entry.setTarget(Some(target)) };
                menu.setAutoenablesItems(false);
                menu.addItem(&entry);
            }
        }
    }
}

pub struct TargetIvars {
    owner: std::rc::Weak<WkPage>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements. No Drop impl.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = TargetIvars]
    struct Target;

    unsafe impl NSObjectProtocol for Target {}

    impl Target {
        #[unsafe(method(run:))]
        fn run(&self, sender: &NSMenuItem) {
            let Some(owner) = self.ivars().owner.upgrade() else { return };
            let event = {
                let st = owner.st.borrow();
                let Some(action) = st.menu_actions.get(sender.tag() as usize) else { return };
                let (checked, was_checked) = implied_check(&action.item);
                ContextMenuItemClicked {
                    id: action.item.id.clone(),
                    page_url: st.committed_url.clone(),
                    link_url: action.hit.link.clone(),
                    image_url: action.hit.image.clone().filter(|i| i != "about:image"),
                    selection_text: action.hit.selection.clone().filter(|s| !s.is_empty()),
                    editable: action.hit.editable,
                    checked,
                    was_checked,
                }
            };
            owner.st.borrow_mut().last_hit = Some((Instant::now() - std::time::Duration::from_secs(60), Hit::default()));
            owner.page.emit(event);
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker, owner: std::rc::Weak<WkPage>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars { owner });
        unsafe { msg_send![super(this), init] }
    }
}
