//! `form()` on AppKit: a SwiftUI `Form` in the grouped style from `NDSwift` (`Form.swift`),
//! whose rows hold the rows' nd controls. The Swift functions are looked up at runtime, since
//! only apps that run `nd::build::app()` link `NDSwift`; without them the form is an
//! `NSGridView` of label and control columns, the plain AppKit settings layout.

use std::ffi::{CString, c_char, c_void};
use std::sync::OnceLock;

use nd_components::{FormProps, FormRowProps, FormSectionProps};
use nd_core::NodeId;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSFont, NSGridCellPlacement, NSGridView, NSTextAlignment, NSTextField, NSView,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSRange, NSString};

type Create = unsafe extern "C" fn() -> *mut c_void;
type Set = unsafe extern "C" fn(*mut c_void, *const c_char);
type SetSlot = unsafe extern "C" fn(*mut c_void, i64, *mut c_void, f64, f64);

struct Swift {
    create: Create,
    set: Set,
    set_slot: SetSlot,
}

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// `RTLD_DEFAULT` from `<dlfcn.h>`: search every image loaded in the process.
const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;

fn swift() -> Option<&'static Swift> {
    static SWIFT: OnceLock<Option<Swift>> = OnceLock::new();
    SWIFT
        .get_or_init(|| {
            let find = |name: &str| {
                let name = CString::new(name).expect("symbol name");
                let ptr = unsafe { dlsym(RTLD_DEFAULT, name.as_ptr()) };
                (!ptr.is_null()).then_some(ptr)
            };
            let (create, set, set_slot) =
                (find("nd_swift_form_create")?, find("nd_swift_form_set")?, find("nd_swift_form_set_slot")?);
            // SAFETY: the symbols are NDSwift's `@_cdecl` functions with these signatures.
            unsafe {
                Some(Swift {
                    create: std::mem::transmute::<*mut c_void, Create>(create),
                    set: std::mem::transmute::<*mut c_void, Set>(set),
                    set_slot: std::mem::transmute::<*mut c_void, SetSlot>(set_slot),
                })
            }
        })
        .as_ref()
}

/// One row as the backend assembles it: its node, props and the control's view.
pub(crate) struct RowSpec {
    pub node: NodeId,
    pub props: FormRowProps,
    pub view: Option<Retained<NSView>>,
    /// Width and height the control's style sets, zero where it sets none.
    pub size: (f64, f64),
}

pub(crate) struct SectionSpec {
    pub node: NodeId,
    pub props: FormSectionProps,
    pub rows: Vec<RowSpec>,
}

fn json(form: &FormProps, sections: &[SectionSpec]) -> String {
    let opt = |s: &Option<nd_ui::prelude::SharedString>| match s {
        Some(s) => serde_json::Value::String(s.to_string()),
        None => serde_json::Value::Null,
    };
    let value: Vec<serde_json::Value> = sections
        .iter()
        .map(|s| {
            serde_json::json!({
                "id": s.node.0,
                "title": opt(&s.props.title),
                "footer": opt(&s.props.footer),
                "rows": s.rows.iter().map(|r| serde_json::json!({
                    "id": r.node.0,
                    "label": r.props.label.to_string(),
                    "help": opt(&r.props.help),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::json!({ "title": opt(&form.title), "pinned": form.title_pinned, "sections": value }).to_string()
}

enum Body {
    Swift(Retained<NSView>),
    Grid(Option<Retained<NSGridView>>),
}

pub(crate) struct FormNative {
    pub outer: Retained<NSView>,
    body: Body,
    pub sections: Vec<NodeId>,
    pub props: FormProps,
}

fn fill(view: &NSView, parent: &NSView) {
    view.setFrame(parent.bounds());
    view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    parent.addSubview(view);
}

impl FormNative {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let outer = NSView::new(mtm);
        let body = match swift() {
            Some(swift) => {
                // SAFETY: returns a +1 retained NSHostingView, created on the main thread.
                let raw = unsafe { (swift.create)() };
                let view = unsafe { Retained::from_raw(raw.cast::<NSView>()) }.expect("NDSwift returns a view");
                fill(&view, &outer);
                Body::Swift(view)
            }
            None => Body::Grid(None),
        };
        Self { outer, body, sections: Vec::new(), props: FormProps::default() }
    }

    /// Shows `sections` and their controls.
    pub fn apply(&mut self, mtm: MainThreadMarker, sections: &[SectionSpec]) {
        match &mut self.body {
            Body::Swift(view) => {
                let swift = swift().expect("resolved at create");
                let raw: *mut c_void = Retained::as_ptr(view).cast_mut().cast();
                let text = CString::new(json(&self.props, sections)).expect("no NUL in JSON");
                unsafe { (swift.set)(raw, text.as_ptr()) };
                for row in sections.iter().flat_map(|s| &s.rows) {
                    let child =
                        row.view.as_ref().map_or(std::ptr::null_mut(), |v| Retained::as_ptr(v).cast_mut().cast());
                    unsafe { (swift.set_slot)(raw, row.node.0 as i64, child, row.size.0, row.size.1) };
                }
            }
            Body::Grid(grid) => {
                if let Some(old) = grid.take() {
                    old.removeFromSuperview();
                }
                let fresh = NSGridView::gridViewWithNumberOfColumns_rows(2, 0, mtm);
                fresh.columnAtIndex(0).setXPlacement(NSGridCellPlacement::Trailing);
                if let Some(title) = &self.props.title {
                    let label = NSTextField::labelWithString(&NSString::from_str(title), mtm);
                    label.setFont(Some(&NSFont::boldSystemFontOfSize(NSFont::systemFontSize() * 2.)));
                    let row = fresh.addRowWithViews(&NSArray::from_retained_slice(&[
                        Retained::into_super(Retained::into_super(label)),
                        NSView::new(mtm),
                    ]));
                    let index = fresh.indexOfRow(&row);
                    fresh
                        .mergeCellsInHorizontalRange_verticalRange(NSRange::new(0, 2), NSRange::new(index as usize, 1));
                }
                for section in sections {
                    if let Some(title) = &section.props.title {
                        let label = NSTextField::labelWithString(&NSString::from_str(title), mtm);
                        label.setFont(Some(&NSFont::boldSystemFontOfSize(NSFont::systemFontSize())));
                        let empty = NSView::new(mtm);
                        let row = fresh.addRowWithViews(&NSArray::from_retained_slice(&[
                            Retained::into_super(Retained::into_super(label)),
                            empty,
                        ]));
                        let index = fresh.indexOfRow(&row);
                        fresh.mergeCellsInHorizontalRange_verticalRange(
                            NSRange::new(0, 2),
                            NSRange::new(index as usize, 1),
                        );
                    }
                    for row in &section.rows {
                        let label = NSTextField::labelWithString(&NSString::from_str(&row.props.label), mtm);
                        label.setAlignment(NSTextAlignment::Right);
                        let control = match &row.view {
                            Some(v) => {
                                v.removeFromSuperview();
                                v.clone()
                            }
                            None => NSView::new(mtm),
                        };
                        fresh.addRowWithViews(&NSArray::from_retained_slice(&[
                            Retained::into_super(Retained::into_super(label)),
                            control,
                        ]));
                    }
                }
                fill(&fresh, &self.outer);
                *grid = Some(fresh);
            }
        }
    }
}
