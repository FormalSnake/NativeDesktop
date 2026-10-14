//! `status_page()` on AppKit: SwiftUI's `ContentUnavailableView` from `NDSwift`
//! (`StatusPage.swift`) in an `NSHostingView`, which Apple restyles with the OS. The Swift
//! functions are looked up at runtime, since only apps that run `nd::build::app()` link
//! `NDSwift`; without them the page is a centred `NSStackView` of the symbol, the title and the
//! description in the system text styles.

use std::ffi::{CString, c_char, c_void};
use std::sync::OnceLock;

use nd_components::StatusPageProps;
use nd_core::Size;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSColor, NSFont, NSFontTextStyle, NSFontTextStyleBody, NSFontTextStyleLargeTitle, NSFontTextStyleTitle2,
    NSImageSymbolConfiguration, NSImageView, NSLayoutAttribute, NSStackView, NSTextAlignment, NSTextField,
    NSUserInterfaceLayoutOrientation, NSView,
};
use objc2_foundation::{MainThreadMarker, NSString};

type Create = unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, bool) -> *mut c_void;
type Update = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, *const c_char, bool);
type Measure = unsafe extern "C" fn(*mut c_void, f64, *mut f64, *mut f64);

struct Swift {
    create: Create,
    update: Update,
    measure: Measure,
}

fn swift() -> Option<&'static Swift> {
    static SWIFT: OnceLock<Option<Swift>> = OnceLock::new();
    SWIFT
        .get_or_init(|| {
            let (create, update, measure) = (
                crate::swift::lookup("nd_swift_status_page_create")?,
                crate::swift::lookup("nd_swift_status_page_update")?,
                crate::swift::lookup("nd_swift_status_page_measure")?,
            );
            // SAFETY: the symbols are NDSwift's `@_cdecl` functions with these signatures.
            unsafe {
                Some(Swift {
                    create: std::mem::transmute::<*mut c_void, Create>(create),
                    update: std::mem::transmute::<*mut c_void, Update>(update),
                    measure: std::mem::transmute::<*mut c_void, Measure>(measure),
                })
            }
        })
        .as_ref()
}

/// The C strings one call passes: title, SF Symbol name, description.
struct Args {
    title: CString,
    symbol: Option<CString>,
    detail: Option<CString>,
}

impl Args {
    fn of(props: &StatusPageProps) -> Self {
        let c = |s: &str| CString::new(s.replace('\0', "")).expect("no NUL left");
        Self {
            title: c(&props.title),
            symbol: props.icon.and_then(|i| crate::icons::symbol_name(i.lucide())).map(c),
            detail: props.description.as_deref().map(c),
        }
    }

    fn symbol(&self) -> *const c_char {
        self.symbol.as_ref().map_or(std::ptr::null(), |s| s.as_ptr())
    }

    fn detail(&self) -> *const c_char {
        self.detail.as_ref().map_or(std::ptr::null(), |s| s.as_ptr())
    }
}

enum Body {
    Hosted(Retained<NSView>),
    Stack {
        stack: Retained<NSStackView>,
        image: Retained<NSImageView>,
        title: Retained<NSTextField>,
        detail: Retained<NSTextField>,
    },
}

pub(crate) struct StatusPageNative {
    body: Body,
}

fn text_style(style: &'static NSFontTextStyle) -> Retained<NSFont> {
    // SAFETY: `style` is one of the AppKit text style constants.
    unsafe { NSFont::preferredFontForTextStyle_options(style, &objc2_foundation::NSDictionary::new()) }
}

impl StatusPageNative {
    pub fn new(mtm: MainThreadMarker, props: &StatusPageProps) -> Self {
        let body = match swift() {
            Some(swift) => {
                let args = Args::of(props);
                // SAFETY: returns a +1 retained NSView (an NSHostingView), created on the main thread.
                let raw = unsafe { (swift.create)(args.title.as_ptr(), args.symbol(), args.detail(), props.compact) };
                Body::Hosted(unsafe { Retained::from_raw(raw.cast::<NSView>()) }.expect("swift returns a view"))
            }
            None => {
                let image = NSImageView::new(mtm);
                let title = NSTextField::wrappingLabelWithString(&NSString::new(), mtm);
                let detail = NSTextField::wrappingLabelWithString(&NSString::new(), mtm);
                detail.setTextColor(Some(&NSColor::secondaryLabelColor()));
                for label in [&title, &detail] {
                    label.setAlignment(NSTextAlignment::Center);
                }
                let stack = NSStackView::new(mtm);
                stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
                stack.setAlignment(NSLayoutAttribute::CenterX);
                stack.addArrangedSubview(&image);
                stack.addArrangedSubview(&title);
                stack.addArrangedSubview(&detail);
                Body::Stack { stack, image, title, detail }
            }
        };
        let this = Self { body };
        if let Body::Stack { .. } = this.body {
            this.update(props);
        }
        this
    }

    pub fn view(&self) -> &NSView {
        match &self.body {
            Body::Hosted(v) => v,
            Body::Stack { stack, .. } => stack,
        }
    }

    pub fn update(&self, props: &StatusPageProps) {
        match &self.body {
            Body::Hosted(view) => {
                let Some(swift) = swift() else { return };
                let args = Args::of(props);
                let raw = Retained::as_ptr(view).cast_mut().cast();
                unsafe { (swift.update)(raw, args.title.as_ptr(), args.symbol(), args.detail(), props.compact) }
            }
            Body::Stack { image, title, detail, .. } => {
                let (symbol_style, title_style) = unsafe {
                    if props.compact {
                        (NSFontTextStyleTitle2, NSFontTextStyleBody)
                    } else {
                        (NSFontTextStyleLargeTitle, NSFontTextStyleTitle2)
                    }
                };
                let symbol = props.icon.and_then(|i| crate::icons::symbol(i.lucide()));
                let config = NSImageSymbolConfiguration::configurationWithTextStyle(symbol_style);
                let symbol = symbol.and_then(|s| s.imageWithSymbolConfiguration(&config));
                image.setImage(symbol.as_deref());
                image.setHidden(symbol.is_none());
                title.setStringValue(&NSString::from_str(&props.title));
                title.setFont(Some(&text_style(title_style)));
                detail.setStringValue(&NSString::from_str(props.description.as_deref().unwrap_or_default()));
                detail.setFont(Some(&text_style(unsafe { NSFontTextStyleBody })));
                detail.setHidden(props.description.is_none());
            }
        }
    }

    /// The page's own size; a definite `width` wraps the description to it.
    pub fn measure(&self, width: Option<f32>) -> Size {
        match &self.body {
            Body::Hosted(view) => {
                let Some(swift) = swift() else { return Size::default() };
                let (mut w, mut h) = (0., 0.);
                let raw = Retained::as_ptr(view).cast_mut().cast();
                unsafe { (swift.measure)(raw, width.map_or(-1., f64::from), &mut w, &mut h) };
                Size { width: w as f32, height: h as f32 }
            }
            Body::Stack { stack, .. } => crate::forms::fitting(stack),
        }
    }
}
