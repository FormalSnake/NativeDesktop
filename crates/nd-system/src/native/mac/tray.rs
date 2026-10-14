//! Menu bar extras as `NSStatusItem`s. An item with a menu opens it on click, as every macOS
//! status item does; one without reports the click itself.

use std::cell::RefCell;
use std::collections::BTreeMap;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSControlStateValueOff, NSControlStateValueOn, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem,
    NSVariableStatusItemLength,
};
use objc2_foundation::{MainThreadMarker, NSData, NSObject, NSObjectProtocol, NSSize, NSString};

use crate::{Error, Result, SystemEvent, Tray, TrayIcon, TrayId, TrayMenuItem};

pub struct TargetIvars {
    tray: TrayId,
    /// Menu item ids, by tag.
    items: RefCell<Vec<String>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = TargetIvars]
    struct NdTrayTarget;

    unsafe impl NSObjectProtocol for NdTrayTarget {}

    impl NdTrayTarget {
        #[unsafe(method(chose:))]
        fn chose(&self, sender: &NSMenuItem) {
            let item = usize::try_from(sender.tag()).ok().and_then(|i| self.ivars().items.borrow().get(i).cloned());
            if item.is_some() {
                super::emitter().emit(SystemEvent::Tray { tray: self.ivars().tray.clone(), item });
            }
        }

        #[unsafe(method(clicked:))]
        fn clicked(&self, _sender: &AnyObject) {
            super::emitter().emit(SystemEvent::Tray { tray: self.ivars().tray.clone(), item: None });
        }
    }
);

impl NdTrayTarget {
    fn new(mtm: MainThreadMarker, tray: TrayId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars { tray, items: RefCell::default() });
        unsafe { msg_send![super(this), init] }
    }
}

struct Item {
    status: Retained<NSStatusItem>,
    /// Menu items and the button hold their target weakly.
    _target: Retained<NdTrayTarget>,
}

thread_local! {
    static ITEMS: RefCell<BTreeMap<String, Item>> = const { RefCell::new(BTreeMap::new()) };
}

fn image(icon: &TrayIcon) -> Result<Retained<NSImage>> {
    let image = match icon {
        TrayIcon::Named(name) => {
            let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)
                .ok_or_else(|| Error::Invalid(format!("no SF Symbol named {name}")))?;
            image.setTemplate(true);
            image
        }
        TrayIcon::Png { bytes, template } => {
            let data = NSData::with_bytes(bytes);
            let image = NSImage::initWithData(NSImage::alloc(), &data)
                .ok_or_else(|| Error::Invalid("tray icon is not an image".into()))?;
            image.setTemplate(*template);
            // The menu bar is 22 points tall; its icons are drawn at 18.
            image.setSize(NSSize::new(18., 18.));
            image
        }
    };
    Ok(image)
}

fn build_menu(mtm: MainThreadMarker, target: &NdTrayTarget, items: &[TrayMenuItem]) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    for item in items {
        let ns = match item {
            TrayMenuItem::Separator => NSMenuItem::separatorItem(mtm),
            TrayMenuItem::Submenu { label, items } => {
                let ns = menu_item(mtm, label, None);
                ns.setSubmenu(Some(&build_menu(mtm, target, items)));
                ns
            }
            TrayMenuItem::Item { id, label, enabled, checked } => {
                let ns = menu_item(mtm, label, Some(sel!(chose:)));
                let mut ids = target.ivars().items.borrow_mut();
                ns.setTag(ids.len() as isize);
                ids.push(id.clone());
                // SAFETY: the target outlives the menu (both live in `ITEMS`).
                unsafe { ns.setTarget(Some(target)) };
                ns.setEnabled(*enabled);
                if let Some(checked) = checked {
                    ns.setState(if *checked { NSControlStateValueOn } else { NSControlStateValueOff });
                }
                ns
            }
        };
        menu.addItem(&ns);
    }
    menu
}

fn menu_item(mtm: MainThreadMarker, title: &str, action: Option<objc2::runtime::Sel>) -> Retained<NSMenuItem> {
    // SAFETY: a plain title and an action the target implements.
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            action,
            &NSString::new(),
        )
    }
}

pub fn set(mtm: MainThreadMarker, tray: Tray) -> Result<()> {
    let image = tray.icon.as_ref().map(image).transpose()?;
    let target = NdTrayTarget::new(mtm, tray.id.clone());
    let status = ITEMS.with_borrow_mut(|items| match items.remove(&tray.id.0) {
        Some(existing) => existing.status,
        None => NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength),
    });
    if let Some(button) = status.button(mtm) {
        button.setImage(image.as_deref());
        button.setTitle(&NSString::from_str(tray.title.as_deref().unwrap_or("")));
        button.setToolTip(tray.tooltip.as_deref().map(NSString::from_str).as_deref());
        // SAFETY: the target outlives the button's use of it (both live in `ITEMS`).
        unsafe {
            button.setTarget(Some(&target));
            button.setAction(if tray.menu.is_empty() { Some(sel!(clicked:)) } else { None });
        }
    }
    status.setMenu((!tray.menu.is_empty()).then(|| build_menu(mtm, &target, &tray.menu)).as_deref());
    ITEMS.with_borrow_mut(|items| items.insert(tray.id.0, Item { status, _target: target }));
    Ok(())
}

pub fn remove(_mtm: MainThreadMarker, id: &TrayId) -> Result<()> {
    let item = ITEMS.with_borrow_mut(|items| items.remove(&id.0)).ok_or(Error::NotFound)?;
    NSStatusBar::systemStatusBar().removeStatusItem(&item.status);
    Ok(())
}

/// The item's button title and top-level menu titles, read back off AppKit.
pub fn probe(mtm: MainThreadMarker, id: &TrayId) -> Option<(String, Vec<String>)> {
    ITEMS.with_borrow(|items| {
        let item = items.get(&id.0)?;
        let title = item.status.button(mtm).map(|b| b.title().to_string()).unwrap_or_default();
        let menu = item.status.menu(mtm).map(|m| m.itemArray().iter().map(|i| i.title().to_string()).collect());
        Some((title, menu.unwrap_or_default()))
    })
}

/// Chooses the top-level menu item at `index` as a click would, or clicks the button when
/// `index` is `None`.
pub fn perform(mtm: MainThreadMarker, id: &TrayId, index: Option<usize>) -> bool {
    ITEMS.with_borrow(|items| {
        let Some(item) = items.get(&id.0) else { return false };
        match index {
            Some(i) => item.status.menu(mtm).is_some_and(|m| {
                let ok = (i as isize) < m.numberOfItems();
                if ok {
                    m.performActionForItemAtIndex(i as isize);
                }
                ok
            }),
            None => item.status.button(mtm).is_some_and(|b| {
                // SAFETY: a nil sender, as a click from AppKit itself may pass.
                unsafe { b.performClick(None) };
                true
            }),
        }
    })
}
