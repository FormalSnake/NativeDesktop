//! Tier 2: the settings window on macOS. Its rows are a SwiftUI `Form` in the grouped style
//! (this example links `NDSwift` through `nd::build::app()`), each row holding the app's native
//! control; its tabs are the toolbar's own selectable items, and picking one swaps the rows.
//! `ND_CHROME_SHOT=<dir>` screenshots the window there. Prints `ND_SETTINGS_OK`.

#[cfg_attr(not(target_os = "macos"), allow(clippy::needless_return))]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let name = "gate_m3::settings_window";
    if args.iter().any(|a| a == "--list") {
        if !args.iter().any(|a| a == "--ignored") && cfg!(target_os = "macos") {
            println!("{name}: test");
        }
        return;
    }
    let filters: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with('-')).collect();
    if args.iter().any(|a| a == "--ignored")
        || !(filters.is_empty() || filters.iter().any(|f| name.contains(f.as_str())))
    {
        return;
    }
    #[cfg(target_os = "macos")]
    settings::run();
}

#[cfg(target_os = "macos")]
mod settings {
    use std::time::{Duration, Instant};

    use nd::appkit::AppHost;
    use nd::appkit::objc2::Message;
    use nd::appkit::objc2::rc::Retained;
    use nd::appkit::objc2_app_kit::{NSPopUpButton, NSView, NSWindow, NSWindowToolbarStyle};

    struct Checks(u32);

    impl Checks {
        fn check(&mut self, ok: bool, what: &str) {
            println!("{} {what}", if ok { "ok  " } else { "FAIL" });
            if !ok {
                self.0 += 1;
            }
        }
    }

    fn pump(host: &AppHost, for_: Duration) {
        let end = Instant::now() + for_;
        while Instant::now() < end {
            host.platform().pump(Duration::from_millis(10), Duration::from_millis(50));
            host.update(|app| {
                if app.needs_frame() {
                    app.frame();
                }
            });
        }
    }

    fn find(view: &NSView, f: &mut dyn FnMut(&NSView) -> bool) -> Option<Retained<NSView>> {
        if f(view) {
            return Some(view.retain());
        }
        view.subviews().iter().find_map(|s| find(&s, f))
    }

    fn class_has(view: &NSView, name: &str) -> bool {
        view.class().name().to_str().unwrap_or("").contains(name)
    }

    fn shot(window: &NSWindow, name: &str) {
        let Some(dir) = std::env::var_os("ND_CHROME_SHOT") else { return };
        let path = std::path::Path::new(&dir).join(format!("{name}.png"));
        let _ = std::process::Command::new("/usr/sbin/screencapture")
            .args(["-x", "-o", &format!("-l{}", window.windowNumber())])
            .arg(&path)
            .status();
        println!("     screenshot {}", path.display());
    }

    pub fn run() -> ! {
        let _lock = ndshell::lock::window_test("gate_m3::settings_window");
        let dir = std::env::temp_dir().join(format!("nd-settings-{}", std::process::id()));
        unsafe {
            std::env::set_var("ND_APP_DATA_DIR", &dir);
            std::env::set_var("ND_THEME_TOML", "");
            std::env::set_var("ND_THEME_JSON", "");
        }
        let host = chrome::app().host(|cx| {
            chrome::init(cx);
        });
        pump(&host, Duration::from_millis(500));
        let settings = host.update(chrome::open_settings_window).expect("settings window");
        pump(&host, Duration::from_millis(800));
        let window = host.ns_window(settings.id()).expect("window");
        let content = window.contentView().expect("content");
        let mut c = Checks(0);

        c.check(window.toolbarStyle() == NSWindowToolbarStyle::Preference, "the settings window is preference-styled");
        let selected = window.toolbar().and_then(|t| t.selectedItemIdentifier()).map(|i| i.to_string());
        c.check(selected.as_deref() == Some("nd.tab.general"), &format!("the General tab is selected ({selected:?})"));
        let hosting = find(&content, &mut |v| class_has(v, "NDFormHost"));
        c.check(hosting.is_some(), "the rows are NDSwift's SwiftUI form");
        let popup = hosting.as_ref().and_then(|h| find(h, &mut |v| v.downcast_ref::<NSPopUpButton>().is_some()));
        c.check(popup.is_some(), "the Density row holds the app's NSPopUpButton inside the form");
        shot(&window, "settings-general");

        // Picking a tab is the toolbar item's own action.
        let accounts = window
            .toolbar()
            .and_then(|t| t.items().iter().find(|i| i.itemIdentifier().to_string() == "nd.tab.accounts"));
        if let Some(item) = &accounts {
            let (Some(target), Some(action)) = (item.target(), item.action()) else {
                panic!("tab without action: target {:?} action {:?}", item.target().is_some(), item.action())
            };
            unsafe { host.platform().app().sendAction_to_from(action, Some(&target), Some(&**item)) };
        }
        c.check(accounts.is_some(), "the Accounts tab is a toolbar item");
        pump(&host, Duration::from_millis(500));
        let tab = host.update(|cx| settings.root().read(cx).tab().clone());
        c.check(tab.as_ref() == "accounts", &format!("picking it reaches the app ({tab})"));
        let selected = window.toolbar().and_then(|t| t.selectedItemIdentifier()).map(|i| i.to_string());
        c.check(selected.as_deref() == Some("nd.tab.accounts"), "the toolbar shows the new selection");
        let popup = find(&content, &mut |v| v.downcast_ref::<NSPopUpButton>().is_some());
        c.check(popup.is_none(), "the General rows are gone");
        shot(&window, "settings-accounts");

        let _ = std::fs::remove_dir_all(&dir);
        if c.0 == 0 {
            println!("ND_SETTINGS_OK");
            std::process::exit(0);
        }
        println!("ND_SETTINGS_FAILED {} check(s)", c.0);
        std::process::exit(1);
    }
}
