//! Tier 2: files dropped on an app window reach `on_file_drop` with that window's id and the
//! drop point from the top left. The drag is a stand-in dragging info handed to the real
//! `NdWindow`'s dragging methods, as AppKit's drag session would. Needs no key window and no
//! cursor. Prints `ND_FILE_DROP_OK`.

mod harness;

fn main() {
    #[cfg(target_os = "macos")]
    harness::main("gate_m7::appkit_file_drop", || app::run());
    #[cfg(not(target_os = "macos"))]
    harness::main("gate_m7::appkit_file_drop", || {});
}

#[cfg(target_os = "macos")]
mod app {
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;

    use nd_backend_appkit::objc2::rc::Retained;
    use nd_backend_appkit::objc2::runtime::AnyObject;
    use nd_backend_appkit::objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
    use nd_backend_appkit::objc2_app_kit::{NSDragOperation, NSPasteboard};
    use nd_backend_appkit::objc2_foundation::{
        MainThreadMarker, NSArray, NSObject, NSObjectProtocol, NSPoint, NSString, NSURL,
    };
    use nd_backend_appkit::{AppHost, on_file_drop};
    use nd_ui::WindowId;
    use nd_ui::prelude::*;

    struct Empty;

    impl Render for Empty {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full()
        }
    }

    pub struct DragIvars {
        pasteboard: Retained<NSPasteboard>,
        at: NSPoint,
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[ivars = DragIvars]
        struct FakeDrag;

        unsafe impl NSObjectProtocol for FakeDrag {}

        // The two NSDraggingInfo methods the window reads; declaring the whole protocol would
        // require all of them.
        impl FakeDrag {
            #[unsafe(method_id(draggingPasteboard))]
            fn dragging_pasteboard(&self) -> Retained<NSPasteboard> {
                self.ivars().pasteboard.clone()
            }

            #[unsafe(method(draggingLocation))]
            fn dragging_location(&self) -> NSPoint {
                self.ivars().at
            }
        }
    );

    fn drag(mtm: MainThreadMarker, paths: &[PathBuf], at: NSPoint) -> Retained<FakeDrag> {
        let pasteboard = NSPasteboard::pasteboardWithUniqueName();
        pasteboard.clearContents();
        if !paths.is_empty() {
            let urls: Vec<Retained<NSURL>> =
                paths.iter().map(|p| NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()))).collect();
            let objects: Vec<&AnyObject> = urls.iter().map(|u| -> &AnyObject { u }).collect();
            // SAFETY: NSURL implements NSPasteboardWriting.
            let ok: bool = unsafe { msg_send![&*pasteboard, writeObjects: &*NSArray::from_slice(&objects)] };
            assert!(ok, "writing the URLs to the pasteboard");
        }
        let this = FakeDrag::alloc(mtm).set_ivars(DragIvars { pasteboard, at });
        unsafe { msg_send![super(this), init] }
    }

    pub fn run() -> ! {
        let mtm = MainThreadMarker::new().unwrap();
        let mut ids = Vec::new();
        let host = AppHost::new(|cx| {
            for title in ["First", "Second"] {
                let handle = cx.open_window(WindowOptions::main(title).title(title), |_, cx| cx.new(|_| Empty));
                ids.push(handle.id());
            }
        });
        type Dropped = (WindowId, Vec<PathBuf>, (f32, f32));
        let drops: Rc<RefCell<Vec<Dropped>>> = Rc::default();
        let log = drops.clone();
        on_file_drop(move |window, paths, at| log.borrow_mut().push((window, paths, at)));

        let dir = std::env::temp_dir().join(format!("nd-file-drop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("dropped one.txt");
        std::fs::write(&file, "x").unwrap();

        let mut failures = 0;
        let mut check = |ok: bool, what: &str| {
            println!("{} {what}", if ok { "ok  " } else { "FAIL" });
            failures += usize::from(!ok);
        };

        let second = host.ns_window(ids[1]).expect("second window");
        let height = second.contentView().unwrap().frame().size.height;
        let files = drag(mtm, std::slice::from_ref(&file), NSPoint::new(30., height - 40.));
        let info: &AnyObject = &files;
        let operation: NSDragOperation = unsafe { msg_send![&*second, draggingEntered: info] };
        check(operation == NSDragOperation::Copy, "a drag carrying files is accepted as a copy");
        let accepted: bool = unsafe { msg_send![&*second, performDragOperation: info] };
        check(accepted, "the drop is taken");
        let got = drops.borrow().clone();
        check(got.len() == 1, &format!("one drop reported ({got:?})"));
        if let Some((window, paths, at)) = got.first() {
            check(*window == ids[1], &format!("with the window it landed on ({window:?}, not {:?})", ids[0]));
            check(paths == std::slice::from_ref(&file), &format!("with its paths ({paths:?})"));
            check(*at == (30., 40.), &format!("at the point from the top left ({at:?})"));
        }

        let text_only = drag(mtm, &[], NSPoint::new(0., 0.));
        let info: &AnyObject = &text_only;
        let operation: NSDragOperation = unsafe { msg_send![&*second, draggingEntered: info] };
        check(operation == NSDragOperation::None, "a drag without files is refused");
        let accepted: bool = unsafe { msg_send![&*second, performDragOperation: info] };
        check(!accepted && drops.borrow().len() == 1, "and reports nothing");

        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_dir(&dir);
        drop(host);
        if failures == 0 {
            println!("ND_FILE_DROP_OK");
            std::process::exit(0);
        }
        std::process::exit(1);
    }
}
