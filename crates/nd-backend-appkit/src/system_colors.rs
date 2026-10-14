//! The macOS preset's inputs, read from AppKit (ARCHITECTURE.md 3.10): semantic colours resolved
//! under both appearances, text style sizes, control heights and reduce motion. Native views
//! never use these; they resolve their own colours at draw time. Composed content reads them as
//! tokens, and [`AppearanceObserver`] re-reads them when the user changes appearance or accent.

use std::cell::Cell;

use block2::RcBlock;
use nd_theme::{OsTheme, PaletteRefinement, Scheme, SystemMetrics};
use nd_ui::prelude::{Hsla, Rgba};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication, NSButton, NSColor, NSColorSpace,
    NSControlSize, NSFont, NSFontTextStyle, NSFontTextStyleBody, NSFontTextStyleCaption1, NSFontTextStyleFootnote,
    NSFontTextStyleHeadline, NSFontTextStyleLargeTitle, NSFontTextStyleTitle1, NSSystemColorsDidChangeNotification,
    NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSDictionary, NSDistributedNotificationCenter, NSNotification, NSNotificationCenter,
    NSObject, NSObjectProtocol, NSString,
};

pub(crate) fn hsla(color: Retained<NSColor>) -> Option<Hsla> {
    let c = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    Some(
        Rgba {
            r: c.redComponent() as f32,
            g: c.greenComponent() as f32,
            b: c.blueComponent() as f32,
            a: c.alphaComponent() as f32,
        }
        .into(),
    )
}

/// The semantic colour behind each base token.
fn palette() -> PaletteRefinement {
    PaletteRefinement {
        surface: hsla(NSColor::windowBackgroundColor()),
        surface_sidebar: hsla(NSColor::windowBackgroundColor()),
        surface_raised: hsla(NSColor::controlBackgroundColor()),
        surface_overlay: hsla(NSColor::controlBackgroundColor()),
        text_primary: hsla(NSColor::labelColor()),
        text_secondary: hsla(NSColor::secondaryLabelColor()),
        text_tertiary: hsla(NSColor::tertiaryLabelColor()),
        accent: hsla(NSColor::controlAccentColor()),
        on_accent: hsla(NSColor::alternateSelectedControlTextColor()),
        accent_container: hsla(NSColor::selectedTextBackgroundColor()),
        on_accent_container: hsla(NSColor::labelColor()),
        separator: hsla(NSColor::separatorColor()),
        selection: hsla(NSColor::selectedContentBackgroundColor()),
        danger: hsla(NSColor::systemRedColor()),
        success: hsla(NSColor::systemGreenColor()),
        warning: hsla(NSColor::systemOrangeColor()),
    }
}

fn under(appearance: &NSAppearance) -> PaletteRefinement {
    let out = Cell::new(None);
    let block = RcBlock::new(|| out.set(Some(palette())));
    appearance.performAsCurrentDrawingAppearance(&block);
    out.take().expect("the block ran synchronously")
}

/// The scheme `appearance` resolves to.
pub fn scheme_of(appearance: &NSAppearance) -> Scheme {
    let names = NSArray::from_slice(&[unsafe { NSAppearanceNameAqua }, unsafe { NSAppearanceNameDarkAqua }]);
    match appearance.bestMatchFromAppearancesWithNames(&names) {
        Some(name) if &*name == unsafe { NSAppearanceNameDarkAqua } => Scheme::Dark,
        _ => Scheme::Light,
    }
}

/// The named system appearance for `scheme`.
pub fn appearance_for(scheme: Scheme) -> Option<Retained<NSAppearance>> {
    NSAppearance::appearanceNamed(match scheme {
        Scheme::Light => unsafe { NSAppearanceNameAqua },
        Scheme::Dark => unsafe { NSAppearanceNameDarkAqua },
    })
}

fn text_size(style: &NSFontTextStyle) -> Option<f32> {
    let font = unsafe { NSFont::preferredFontForTextStyle_options(style, &NSDictionary::new()) };
    Some(font.pointSize() as f32)
}

fn control_height(mtm: MainThreadMarker, size: NSControlSize) -> Option<f32> {
    let probe = unsafe { NSButton::buttonWithTitle_target_action(&NSString::from_str("X"), None, None, mtm) };
    probe.setControlSize(size);
    Some(probe.fittingSize().height as f32).filter(|h| *h > 0.)
}

/// Everything the macOS preset reads from the OS, for both schemes.
pub fn os_theme(mtm: MainThreadMarker) -> OsTheme {
    let mut os = OsTheme {
        scheme: Some(scheme_of(&NSApplication::sharedApplication(mtm).effectiveAppearance())),
        ..OsTheme::default()
    };
    for scheme in [Scheme::Light, Scheme::Dark] {
        if let Some(appearance) = appearance_for(scheme) {
            *os.palettes.get_mut(scheme) = under(&appearance);
        }
    }
    os.metrics = SystemMetrics {
        caption: text_size(unsafe { NSFontTextStyleCaption1 }),
        footnote: text_size(unsafe { NSFontTextStyleFootnote }),
        body: text_size(unsafe { NSFontTextStyleBody }),
        headline: text_size(unsafe { NSFontTextStyleHeadline }),
        title: text_size(unsafe { NSFontTextStyleTitle1 }),
        large_title: text_size(unsafe { NSFontTextStyleLargeTitle }),
        control_small: control_height(mtm, NSControlSize::Small),
        control_regular: control_height(mtm, NSControlSize::Regular),
        control_large: control_height(mtm, NSControlSize::Large),
        reduce_motion: Some(NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()),
    };
    os
}

pub struct ObserverIvars {
    callback: Box<dyn Fn()>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ObserverIvars]
    struct NdAppearanceObserver;

    unsafe impl NSObjectProtocol for NdAppearanceObserver {}

    impl NdAppearanceObserver {
        #[unsafe(method(ndAppearanceChanged:))]
        fn changed(&self, _n: &NSNotification) {
            // NSApp's effective appearance follows the distributed notification a turn later.
            let this: Retained<Self> = unsafe { Retained::retain(self as *const Self as *mut Self) }.expect("self");
            crate::events::defer(move || (this.ivars().callback)());
        }
    }
);

/// Calls back after the user changes appearance, accent, highlight colour or reduce motion.
/// Stops observing when dropped.
pub struct AppearanceObserver {
    observer: Retained<NdAppearanceObserver>,
}

impl AppearanceObserver {
    pub fn new(mtm: MainThreadMarker, callback: impl Fn() + 'static) -> Self {
        let observer = NdAppearanceObserver::alloc(mtm).set_ivars(ObserverIvars { callback: Box::new(callback) });
        let observer: Retained<NdAppearanceObserver> = unsafe { msg_send![super(observer), init] };
        let target: &AnyObject = &observer;
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                target,
                sel!(ndAppearanceChanged:),
                Some(NSSystemColorsDidChangeNotification),
                None,
            );
            NSWorkspace::sharedWorkspace().notificationCenter().addObserver_selector_name_object(
                target,
                sel!(ndAppearanceChanged:),
                Some(NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification),
                None,
            );
            let distributed = NSDistributedNotificationCenter::defaultCenter();
            for name in ["AppleInterfaceThemeChangedNotification", "AppleColorPreferencesChangedNotification"] {
                distributed.addObserver_selector_name_object(
                    target,
                    sel!(ndAppearanceChanged:),
                    Some(&NSString::from_str(name)),
                    None,
                );
            }
        }
        Self { observer }
    }
}

impl Drop for AppearanceObserver {
    fn drop(&mut self) {
        let target: &AnyObject = &self.observer;
        unsafe {
            NSNotificationCenter::defaultCenter().removeObserver(target);
            NSWorkspace::sharedWorkspace().notificationCenter().removeObserver(target);
            NSDistributedNotificationCenter::defaultCenter().removeObserver(target);
        }
    }
}
