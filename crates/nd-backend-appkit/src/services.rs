//! [`ChromeServices`] on AppKit: the global `NSMenu` menu bar with the app menu roles, context
//! menus as `NSMenu`, `NSAlert` sheets and `NSOpenPanel`/`NSSavePanel` (ARCHITECTURE.md 3.6).
//! Menu items carry nd actions and show the key equivalent of the binding that dispatches the
//! same action, so a menu and its shortcut never disagree. Every native callback hands its
//! result to the app through [`defer_with_app`], never directly (3.13).

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use block2::RcBlock;
use nd_components::services::{AlertCallback, ChromeServices, OpenCallback, SaveCallback};
use nd_components::{
    AboutPanel, Alert, AlertPresentation, AlertResponse, AlertStyle, ButtonRole, ContextMenu, Menu, MenuItem, MenuKind,
    MenuRole, OpenDialog, SaveDialog,
};
use nd_core::{NodeId, WindowId};
use nd_ui::{Action, App, KeyBinding};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAboutPanelOptionApplicationName, NSAboutPanelOptionApplicationVersion, NSAboutPanelOptionCredits,
    NSAboutPanelOptionVersion, NSAlert, NSAlertStyle, NSApplication, NSColor, NSControlStateValueOn,
    NSEventModifierFlags, NSFont, NSFontAttributeName, NSForegroundColorAttributeName, NSLinkAttributeName, NSMenu,
    NSMenuItem, NSOpenPanel, NSSavePanel,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSAttributedStringKey, NSDictionary, NSMutableAttributedString, NSObject,
    NSObjectProtocol, NSPoint, NSString, NSURL,
};

use crate::AppKitBackend;
use crate::app_host::{defer_with_app, key_window};

pub struct TargetIvars {
    actions: RefCell<Vec<Box<dyn Action>>>,
    /// The window a context menu belongs to; the key window for the menu bar.
    window: Cell<Option<WindowId>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = TargetIvars]
    struct NdMenuTarget;

    unsafe impl NSObjectProtocol for NdMenuTarget {}

    impl NdMenuTarget {
        #[unsafe(method(fire:))]
        fn fire(&self, sender: &NSMenuItem) {
            let Ok(index) = usize::try_from(sender.tag()) else { return };
            let Some(action) = self.ivars().actions.borrow().get(index).map(|a| a.boxed_clone()) else { return };
            let Some(window) = self.ivars().window.get().or_else(key_window) else { return };
            defer_with_app(move |app| {
                app.dispatch_action(window, &*action);
            });
        }
    }
);

impl NdMenuTarget {
    fn new(mtm: MainThreadMarker, window: Option<WindowId>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars { actions: RefCell::default(), window: Cell::new(window) });
        unsafe { msg_send![super(this), init] }
    }

    fn add(&self, action: Box<dyn Action>) -> isize {
        let mut actions = self.ivars().actions.borrow_mut();
        actions.push(action);
        actions.len() as isize - 1
    }
}

thread_local! {
    /// Targets AppKit holds weakly: the menu bar's, and the context menu's while it is open.
    static MENU_BAR_TARGET: RefCell<Option<Retained<NdMenuTarget>>> = const { RefCell::new(None) };
}

fn item(mtm: MainThreadMarker, title: &str, action: Option<Sel>, key: &str) -> Retained<NSMenuItem> {
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            action,
            &NSString::from_str(key),
        )
    }
}

fn modifiers(m: &nd_ui::Modifiers) -> NSEventModifierFlags {
    let mut flags = NSEventModifierFlags::empty();
    for (on, flag) in [
        (m.platform, NSEventModifierFlags::Command),
        (m.control, NSEventModifierFlags::Control),
        (m.alt, NSEventModifierFlags::Option),
        (m.shift, NSEventModifierFlags::Shift),
    ] {
        if on {
            flags |= flag;
        }
    }
    flags
}

/// gpui key names to the characters `keyEquivalent` takes.
fn key_equivalent(key: &str) -> Option<String> {
    let named = match key {
        "enter" => "\r",
        "escape" => "\u{1b}",
        "backspace" => "\u{8}",
        "delete" => "\u{7f}",
        "tab" => "\t",
        "space" => " ",
        "up" => "\u{f700}",
        "down" => "\u{f701}",
        "left" => "\u{f702}",
        "right" => "\u{f703}",
        k if k.chars().count() == 1 => return Some(k.to_owned()),
        _ => return None,
    };
    Some(named.to_owned())
}

/// Sets `menu_item`'s key equivalent from the first single-stroke binding of `action`.
fn bind(menu_item: &NSMenuItem, action: &dyn Action, bindings: &[KeyBinding]) {
    let found = bindings.iter().rev().find(|b| b.keystrokes().len() == 1 && b.action().partial_eq(action));
    let Some(binding) = found else { return };
    let stroke = &binding.keystrokes()[0];
    let Some(key) = key_equivalent(stroke.key()) else { return };
    menu_item.setKeyEquivalent(&NSString::from_str(&key));
    menu_item.setKeyEquivalentModifierMask(modifiers(stroke.modifiers()));
}

fn role_item(mtm: MainThreadMarker, role: MenuRole, app_name: &str) -> Retained<NSMenuItem> {
    let cmd = NSEventModifierFlags::Command;
    let (title, action, key, mask) = match role {
        MenuRole::About => (format!("About {app_name}"), sel!(orderFrontStandardAboutPanel:), "", cmd),
        MenuRole::Services => {
            let services = item(mtm, "Services", None, "");
            let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Services"));
            services.setSubmenu(Some(&menu));
            NSApplication::sharedApplication(mtm).setServicesMenu(Some(&menu));
            return services;
        }
        MenuRole::Hide => (format!("Hide {app_name}"), sel!(hide:), "h", cmd),
        MenuRole::HideOthers => {
            ("Hide Others".into(), sel!(hideOtherApplications:), "h", cmd | NSEventModifierFlags::Option)
        }
        MenuRole::ShowAll => ("Show All".into(), sel!(unhideAllApplications:), "", cmd),
        MenuRole::Quit => (format!("Quit {app_name}"), sel!(terminate:), "q", cmd),
        MenuRole::Undo => ("Undo".into(), sel!(undo:), "z", cmd),
        MenuRole::Redo => ("Redo".into(), sel!(redo:), "z", cmd | NSEventModifierFlags::Shift),
        MenuRole::Cut => ("Cut".into(), sel!(cut:), "x", cmd),
        MenuRole::Copy => ("Copy".into(), sel!(copy:), "c", cmd),
        MenuRole::Paste => ("Paste".into(), sel!(paste:), "v", cmd),
        MenuRole::SelectAll => ("Select All".into(), sel!(selectAll:), "a", cmd),
        MenuRole::Minimize => ("Minimize".into(), sel!(performMiniaturize:), "m", cmd),
        MenuRole::Zoom => ("Zoom".into(), sel!(performZoom:), "", cmd),
        MenuRole::BringAllToFront => ("Bring All to Front".into(), sel!(arrangeInFront:), "", cmd),
        MenuRole::ToggleFullScreen => {
            ("Enter Full Screen".into(), sel!(toggleFullScreen:), "f", cmd | NSEventModifierFlags::Control)
        }
    };
    let i = item(mtm, &title, Some(action), key);
    i.setKeyEquivalentModifierMask(mask);
    i
}

struct Builder<'a> {
    mtm: MainThreadMarker,
    target: &'a NdMenuTarget,
    bindings: &'a [KeyBinding],
    app_name: &'a str,
}

impl Builder<'_> {
    fn items(&self, menu: &NSMenu, items: Vec<MenuItem>) {
        for entry in items {
            match entry {
                MenuItem::Separator => menu.addItem(&NSMenuItem::separatorItem(self.mtm)),
                MenuItem::Role(role) => menu.addItem(&role_item(self.mtm, role, self.app_name)),
                MenuItem::Submenu(sub) => {
                    let parent = item(self.mtm, &sub.title, None, "");
                    parent.setSubmenu(Some(&self.menu(sub)));
                    menu.addItem(&parent);
                }
                MenuItem::Action { label, action, icon, destructive: _, enabled, checked } => {
                    let i = item(self.mtm, &label, Some(sel!(fire:)), "");
                    bind(&i, &*action, self.bindings);
                    unsafe { i.setTarget(Some(self.target)) };
                    i.setEnabled(enabled);
                    if checked {
                        i.setState(NSControlStateValueOn);
                    }
                    if let Some(icon) = icon {
                        i.setImage(crate::icons::symbol(icon.lucide()).as_deref());
                    }
                    i.setTag(self.target.add(action));
                    menu.addItem(&i);
                }
            }
        }
    }

    fn menu(&self, menu: Menu) -> Retained<NSMenu> {
        let ns = NSMenu::initWithTitle(NSMenu::alloc(self.mtm), &NSString::from_str(&menu.title));
        ns.setAutoenablesItems(false);
        let role = |r| MenuItem::Role(r);
        let items = match menu.kind {
            MenuKind::App => {
                let mut items = vec![role(MenuRole::About), MenuItem::Separator];
                let own = !menu.items.is_empty();
                items.extend(menu.items);
                if own {
                    items.push(MenuItem::Separator);
                }
                items.extend([
                    role(MenuRole::Services),
                    MenuItem::Separator,
                    role(MenuRole::Hide),
                    role(MenuRole::HideOthers),
                    role(MenuRole::ShowAll),
                    MenuItem::Separator,
                    role(MenuRole::Quit),
                ]);
                // System items use the responder chain; they must autoenable.
                ns.setAutoenablesItems(true);
                items
            }
            MenuKind::Edit => {
                ns.setAutoenablesItems(true);
                let mut items = vec![
                    role(MenuRole::Undo),
                    role(MenuRole::Redo),
                    MenuItem::Separator,
                    role(MenuRole::Cut),
                    role(MenuRole::Copy),
                    role(MenuRole::Paste),
                    role(MenuRole::SelectAll),
                ];
                if !menu.items.is_empty() {
                    items.push(MenuItem::Separator);
                    items.extend(menu.items);
                }
                items
            }
            MenuKind::Window => {
                ns.setAutoenablesItems(true);
                let mut items = vec![role(MenuRole::Minimize), role(MenuRole::Zoom), role(MenuRole::ToggleFullScreen)];
                if !menu.items.is_empty() {
                    items.push(MenuItem::Separator);
                    items.extend(menu.items);
                }
                items.extend([MenuItem::Separator, role(MenuRole::BringAllToFront)]);
                NSApplication::sharedApplication(self.mtm).setWindowsMenu(Some(&ns));
                items
            }
            MenuKind::Help => {
                NSApplication::sharedApplication(self.mtm).setHelpMenu(Some(&ns));
                menu.items
            }
            MenuKind::Normal => menu.items,
        };
        self.items(&ns, items);
        ns
    }
}

fn app_name() -> String {
    objc2_foundation::NSProcessInfo::processInfo().processName().to_string()
}

/// The menu bar from `menus`, with the app, Edit and Window menus added where the app left
/// them out, so the system items (Quit, Copy, Minimize) are always there.
pub fn menu_bar(mtm: MainThreadMarker, mut menus: Vec<Menu>, bindings: &[KeyBinding]) -> Retained<NSMenu> {
    if !menus.iter().any(|m| m.kind == MenuKind::App) {
        menus.insert(0, Menu::app([]));
    }
    if !menus.iter().any(|m| m.kind == MenuKind::Edit) {
        let at = menus.iter().position(|m| m.kind == MenuKind::App).map_or(0, |i| i + 1);
        let after_file = menus.get(at).is_some_and(|m| m.title.as_ref() == "File");
        menus.insert(at + usize::from(after_file), Menu::edit([]));
    }
    if !menus.iter().any(|m| m.kind == MenuKind::Window) {
        let at = menus.iter().position(|m| m.kind == MenuKind::Help).unwrap_or(menus.len());
        menus.insert(at, Menu::window([]));
    }
    let target = NdMenuTarget::new(mtm, None);
    let name = app_name();
    let builder = Builder { mtm, target: &target, bindings, app_name: &name };
    let bar = NSMenu::new(mtm);
    for menu in menus {
        let top = item(mtm, &menu.title, None, "");
        top.setSubmenu(Some(&builder.menu(menu)));
        bar.addItem(&top);
    }
    MENU_BAR_TARGET.with(|t| *t.borrow_mut() = Some(target));
    bar
}

/// Runs `f` once from a block AppKit may only declare as `Fn`.
fn once<A>(f: impl FnOnce(A) + 'static) -> impl Fn(A) + 'static {
    let f = Cell::new(Some(f));
    move |a| {
        if let Some(f) = f.take() {
            f(a);
        }
    }
}

fn ns_window(backend: &AppKitBackend, window: WindowId) -> Option<Retained<objc2_app_kit::NSWindow>> {
    backend.ns_window(window).map(Retained::into_super)
}

/// The AppKit chrome services, installed as the `Chrome` global by `nd`.
pub struct AppKitChrome {
    backend: AppKitBackend,
}

impl AppKitChrome {
    pub fn new(backend: AppKitBackend) -> Self {
        Self { backend }
    }
}

impl ChromeServices for AppKitChrome {
    fn set_menus(&self, cx: &mut App, menus: Vec<Menu>) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let bar = menu_bar(mtm, menus, cx.key_bindings());
        NSApplication::sharedApplication(mtm).setMainMenu(Some(&bar));
    }

    fn show_context_menu(&self, cx: &mut App, window: WindowId, node: NodeId, x: f32, y: f32, menu: ContextMenu) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let Some(view) = self.backend.view(node) else { return };
        let target = NdMenuTarget::new(mtm, Some(window));
        let name = app_name();
        let builder = Builder { mtm, target: &target, bindings: cx.key_bindings(), app_name: &name };
        let ns = NSMenu::new(mtm);
        ns.setAutoenablesItems(false);
        builder.items(&ns, menu.items);
        let y = if menu.above { f64::from(y) - ns.size().height } else { f64::from(y) };
        // Menu tracking is a nested run loop: start it from a later turn, never inside the
        // handler that asked for it (3.13).
        crate::events::defer(move || {
            let _target = target;
            ns.popUpMenuPositioningItem_atLocation_inView(None, NSPoint::new(x.into(), y), Some(&view));
        });
    }

    fn alert(&self, _cx: &mut App, window: WindowId, alert: Alert, done: AlertCallback) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let ns = NSAlert::new(mtm);
        ns.setMessageText(&NSString::from_str(&alert.title));
        if let Some(message) = &alert.message {
            ns.setInformativeText(&NSString::from_str(message));
        }
        ns.setAlertStyle(match alert.style {
            AlertStyle::Informational => NSAlertStyle::Informational,
            AlertStyle::Warning => NSAlertStyle::Warning,
            AlertStyle::Critical => NSAlertStyle::Critical,
        });
        for b in &alert.buttons {
            let button = ns.addButtonWithTitle(&NSString::from_str(&b.label));
            match b.role {
                ButtonRole::Destructive => button.setHasDestructiveAction(true),
                ButtonRole::Cancel => button.setKeyEquivalent(&NSString::from_str("\u{1b}")),
                ButtonRole::Default => {}
            }
        }
        let parent = match alert.presentation {
            AlertPresentation::Sheet => ns_window(&self.backend, window),
            AlertPresentation::Modal => None,
        };
        let finish = once(move |code: isize| {
            // NSAlertFirstButtonReturn is 1000; later buttons count up from it.
            let index = usize::try_from(code - 1000).unwrap_or(0);
            defer_with_app(move |app| done(AlertResponse(index), app));
        });
        crate::events::defer(move || match parent {
            Some(parent) => {
                let block = RcBlock::new(move |code: isize| finish(code));
                ns.beginSheetModalForWindow_completionHandler(&parent, Some(&block));
            }
            None => finish(ns.runModal()),
        });
    }

    fn about(&self, _cx: &mut App, _window: Option<WindowId>, about: AboutPanel) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let mut keys: Vec<&NSString> = Vec::new();
        let mut values: Vec<Retained<AnyObject>> = Vec::new();
        let copyright = NSString::from_str("Copyright");
        let fields = [
            (unsafe { NSAboutPanelOptionApplicationName }, &about.name),
            (unsafe { NSAboutPanelOptionApplicationVersion }, &about.version),
            (unsafe { NSAboutPanelOptionVersion }, &about.build),
            (&*copyright, &about.copyright),
        ];
        for (key, value) in fields {
            if let Some(value) = value {
                keys.push(key);
                values.push(Retained::into_super(Retained::into_super(NSString::from_str(value))));
            }
        }
        if let Some(credits) = credits(&about) {
            keys.push(unsafe { NSAboutPanelOptionCredits });
            values.push(Retained::into_super(Retained::into_super(Retained::into_super(credits))));
        }
        let options = NSDictionary::from_retained_objects(&keys, &values);
        // Ordering a window front runs AppKit callbacks; never under the handler's lease (3.13).
        crate::events::defer(move || unsafe {
            NSApplication::sharedApplication(mtm).orderFrontStandardAboutPanelWithOptions(&options);
        });
    }

    fn open_dialog(&self, _cx: &mut App, window: WindowId, dialog: OpenDialog, done: OpenCallback) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let panel = NSOpenPanel::openPanel(mtm);
        panel.setCanChooseFiles(dialog.files || !dialog.directories);
        panel.setCanChooseDirectories(dialog.directories);
        panel.setAllowsMultipleSelection(dialog.multiple);
        configure(
            &panel,
            dialog.title.as_deref(),
            dialog.prompt.as_deref(),
            dialog.directory.as_deref(),
            &dialog.extensions,
        );
        let parent = ns_window(&self.backend, window);
        let read = panel.clone();
        let finish = once(move |code: isize| {
            let paths = (code == 1).then(|| read.URLs().iter().filter_map(|u| path_of(&u)).collect::<Vec<_>>());
            defer_with_app(move |app| done(paths, app));
        });
        crate::events::defer(move || present(&panel, parent, finish));
    }

    fn save_dialog(&self, _cx: &mut App, window: WindowId, dialog: SaveDialog, done: SaveCallback) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let panel = NSSavePanel::savePanel(mtm);
        configure(
            &panel,
            dialog.title.as_deref(),
            dialog.prompt.as_deref(),
            dialog.directory.as_deref(),
            &dialog.extensions,
        );
        if let Some(name) = &dialog.file_name {
            panel.setNameFieldStringValue(&NSString::from_str(name));
        }
        let parent = ns_window(&self.backend, window);
        let read = panel.clone();
        let finish = once(move |code: isize| {
            let path = (code == 1).then(|| read.URL().and_then(|u| path_of(&u))).flatten();
            defer_with_app(move |app| done(path, app));
        });
        crate::events::defer(move || present(&panel, parent, finish));
    }
}

/// The about panel's credits: the credits line, then the website as a link.
fn credits(about: &AboutPanel) -> Option<Retained<NSMutableAttributedString>> {
    let lines: Vec<&str> = [about.credits.as_deref(), about.website.as_deref()].into_iter().flatten().collect();
    if lines.is_empty() {
        return None;
    }
    let text = NSString::from_str(&lines.join("\n"));
    let font = NSFont::systemFontOfSize(NSFont::smallSystemFontSize());
    let color = NSColor::labelColor();
    let attributes: Retained<NSDictionary<NSAttributedStringKey, AnyObject>> = NSDictionary::from_retained_objects(
        &[unsafe { NSFontAttributeName }, unsafe { NSForegroundColorAttributeName }],
        &[Retained::into_super(Retained::into_super(font)), Retained::into_super(Retained::into_super(color))],
    );
    let credits = unsafe {
        NSMutableAttributedString::initWithString_attributes(
            NSMutableAttributedString::alloc(),
            &text,
            Some(&attributes),
        )
    };
    if let Some(website) = about.website.as_deref()
        && let Some(url) = NSURL::URLWithString(&NSString::from_str(website))
    {
        let range = text.rangeOfString(&NSString::from_str(website));
        unsafe { credits.addAttribute_value_range(NSLinkAttributeName, &url, range) };
    }
    Some(credits)
}

fn path_of(url: &NSURL) -> Option<PathBuf> {
    url.path().map(|p| PathBuf::from(p.to_string()))
}

fn configure(
    panel: &NSSavePanel,
    title: Option<&str>,
    prompt: Option<&str>,
    directory: Option<&std::path::Path>,
    extensions: &[nd_ui::prelude::SharedString],
) {
    if let Some(title) = title {
        panel.setMessage(Some(&NSString::from_str(title)));
    }
    if let Some(prompt) = prompt {
        panel.setPrompt(Some(&NSString::from_str(prompt)));
    }
    if let Some(dir) = directory {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&dir.to_string_lossy()));
        panel.setDirectoryURL(Some(&url));
    }
    if !extensions.is_empty() {
        let types: Vec<Retained<NSString>> = extensions.iter().map(|e| NSString::from_str(e)).collect();
        let array = NSArray::from_retained_slice(&types);
        // `allowedContentTypes` needs UTTypes; the extension list is what apps have.
        let _: () = unsafe { msg_send![panel, setAllowedFileTypes: &*array] };
    }
}

fn present(panel: &NSSavePanel, parent: Option<Retained<objc2_app_kit::NSWindow>>, finish: impl Fn(isize) + 'static) {
    match parent {
        Some(parent) => {
            let block = RcBlock::new(move |code: isize| finish(code));
            panel.beginSheetModalForWindow_completionHandler(&parent, &block);
        }
        None => finish(panel.runModal()),
    }
}
