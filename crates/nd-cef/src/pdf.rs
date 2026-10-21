//! `print_to_pdf`: Chromium lays the page out for print and writes it to a file, answering on
//! the main thread.

use std::cell::RefCell;
use std::collections::HashMap;
use std::os::raw::c_int;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use cef::*;

type Done = Box<dyn FnOnce(Result<(), String>)>;

thread_local! {
    static PENDING: RefCell<HashMap<u64, Done>> = RefCell::default();
}

pub(crate) fn print_to_pdf(host: &BrowserHost, path: &Path, done: Done) {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    PENDING.with(|p| p.borrow_mut().insert(id, done));
    let settings = PdfPrintSettings { print_background: 1, ..Default::default() };
    let path = CefString::from(path.to_string_lossy().as_ref());
    host.print_to_pdf(Some(&path), Some(&settings), Some(&mut NdPdf::new(id)));
}

wrap_pdf_print_callback! {
    struct NdPdf {
        id: u64,
    }

    impl PdfPrintCallback {
        fn on_pdf_print_finished(&self, path: Option<&CefString>, ok: c_int) {
            let Some(done) = PENDING.with(|p| p.borrow_mut().remove(&self.id)) else { return };
            let path = path.map(|p| p.to_string()).unwrap_or_default();
            done(if ok != 0 { Ok(()) } else { Err(format!("Chromium could not write {path}")) });
        }
    }
}
