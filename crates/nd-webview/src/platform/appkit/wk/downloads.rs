//! Downloads through `WKDownload`, owned by the app: WebKit waits for a destination until the
//! app answers [`crate::DownloadRequested`], reports progress, and pauses and resumes through
//! resume data.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use block2::RcBlock;
use objc2::Message;
use objc2::rc::Retained;
use objc2_foundation::{
    MainThreadMarker, NSData, NSError, NSKeyValueObservingOptions, NSObject, NSObjectNSKeyValueObserverRegistration,
    NSProgressReporting, NSString, NSURL, NSURLResponse,
};
use objc2_web_kit::WKDownload;

use super::WkPage;
use super::convert::{ns, url_string};
use crate::events::{DownloadRequested, DownloadState, DownloadUpdated};
use crate::{Download, DownloadControl};

const PROGRESS_KEY: &str = "completedUnitCount";

thread_local! {
    static NEXT: Cell<u64> = const { Cell::new(1) };
    /// Which page each download id belongs to, for progress reports that arrive off the main
    /// thread and come back by id.
    static OWNERS: RefCell<HashMap<u64, Weak<WkPage>>> = RefCell::default();
}

#[derive(Default)]
pub(crate) struct Downloads {
    list: HashMap<u64, Entry>,
}

struct Entry {
    download: Retained<WKDownload>,
    url: String,
    path: Option<PathBuf>,
    decide: Option<RcBlock<dyn Fn(*mut NSURL)>>,
    resume: Option<Retained<NSData>>,
    observing: bool,
    pausing: bool,
    paused: bool,
}

fn id_of(owner: &WkPage, download: &WKDownload) -> Option<u64> {
    owner.st.borrow().downloads.list.iter().find(|(_, e)| std::ptr::eq(&*e.download, download)).map(|(id, _)| *id)
}

/// WebKit turned a navigation (or `start_download`) into `download`.
pub(super) fn adopt(owner: &WkPage, download: &WKDownload) {
    if id_of(owner, download).is_some() {
        return;
    }
    unsafe { download.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*owner.delegate))) };
    let id = NEXT.with(|n| n.replace(n.get() + 1));
    OWNERS.with(|o| o.borrow_mut().insert(id, owner.this.clone()));
    let url =
        unsafe { download.originalRequest() }.and_then(|r| r.URL()).map(|u| url_string(Some(&u))).unwrap_or_default();
    owner.st.borrow_mut().downloads.list.insert(
        id,
        Entry {
            download: download.retain(),
            url,
            path: None,
            decide: None,
            resume: None,
            observing: false,
            pausing: false,
            paused: false,
        },
    );
}

pub(super) fn decide(
    owner: &WkPage,
    download: &WKDownload,
    response: &NSURLResponse,
    suggested: &str,
    done: RcBlock<dyn Fn(*mut NSURL)>,
) {
    adopt(owner, download);
    let Some(id) = id_of(owner, download) else {
        done.call((std::ptr::null_mut(),));
        return;
    };
    let url = {
        let mut st = owner.st.borrow_mut();
        let entry = st.downloads.list.get_mut(&id).expect("adopted");
        entry.decide = Some(done);
        if entry.url.is_empty() {
            entry.url = url_string(response.URL().as_deref());
        }
        entry.url.clone()
    };
    let total = u64::try_from(response.expectedContentLength()).ok();
    let mime = response.MIMEType().map(|m| m.to_string());
    let control = Rc::new(Control { owner: owner.this.clone(), id, decided: Cell::new(false) });
    let download = Download::new(id, url, suggested.to_owned(), mime, total, control);
    owner.page.emit(DownloadRequested { download });
}

fn emit(owner: &WkPage, id: u64, state: DownloadState, error: Option<String>) {
    let event = {
        let st = owner.st.borrow();
        let Some(entry) = st.downloads.list.get(&id) else { return };
        let progress = entry.download.progress();
        DownloadUpdated {
            id,
            state,
            received: progress.completedUnitCount().max(0) as u64,
            total: u64::try_from(progress.totalUnitCount()).ok(),
            path: entry.path.clone(),
            paused: entry.paused,
            error,
        }
    };
    owner.page.emit(event);
}

fn observe(owner: &WkPage, id: u64, on: bool) {
    let mut st = owner.st.borrow_mut();
    let Some(entry) = st.downloads.list.get_mut(&id) else { return };
    if entry.observing == on {
        return;
    }
    entry.observing = on;
    let progress = entry.download.progress();
    let observer: &NSObject = &owner.delegate;
    unsafe {
        if on {
            progress.addObserver_forKeyPath_options_context(
                observer,
                &ns(PROGRESS_KEY),
                NSKeyValueObservingOptions::New,
                id as *mut std::ffi::c_void,
            );
        } else {
            progress.removeObserver_forKeyPath(observer, &ns(PROGRESS_KEY));
        }
    }
}

/// A progress observation fired; NSProgress may report from any thread.
pub(super) fn progress(owner: &WkPage, id: u64) {
    if MainThreadMarker::new().is_some() {
        emit(owner, id, DownloadState::Running, None);
    } else {
        dispatch2::DispatchQueue::main().exec_async(move || {
            if let Some(owner) = OWNERS.with(|o| o.borrow().get(&id).and_then(Weak::upgrade)) {
                emit(&owner, id, DownloadState::Running, None);
            }
        });
    }
}

pub(super) fn finished(owner: &WkPage, download: &WKDownload) {
    let Some(id) = id_of(owner, download) else { return };
    observe(owner, id, false);
    emit(owner, id, DownloadState::Done, None);
    owner.st.borrow_mut().downloads.list.remove(&id);
    OWNERS.with(|o| o.borrow_mut().remove(&id));
}

pub(super) fn failed(owner: &WkPage, download: &WKDownload, error: &NSError, resume: Option<&NSData>) {
    let Some(id) = id_of(owner, download) else { return };
    observe(owner, id, false);
    let pausing = {
        let mut st = owner.st.borrow_mut();
        let entry = st.downloads.list.get_mut(&id).expect("known");
        entry.resume = resume.map(|r| r.retain());
        std::mem::take(&mut entry.pausing)
    };
    if pausing {
        return;
    }
    let cancelled = error.code() == objc2_foundation::NSURLErrorCancelled;
    let state = if cancelled { DownloadState::Cancelled } else { DownloadState::Failed };
    emit(owner, id, state, (!cancelled).then(|| error.localizedDescription().to_string()));
    if cancelled {
        owner.st.borrow_mut().downloads.list.remove(&id);
        OWNERS.with(|o| o.borrow_mut().remove(&id));
    }
}

struct Control {
    owner: Weak<WkPage>,
    id: u64,
    decided: Cell<bool>,
}

impl Control {
    fn with(&self, f: impl FnOnce(&Rc<WkPage>, u64)) {
        if let Some(owner) = self.owner.upgrade() {
            f(&owner, self.id);
        }
    }
}

impl Drop for Control {
    /// Nobody chose a destination: the download is cancelled, as a browser does when its
    /// save dialog is dismissed.
    fn drop(&mut self) {
        if !self.decided.get() {
            self.cancel();
        }
    }
}

impl DownloadControl for Control {
    fn save_to(&self, path: PathBuf) {
        self.decided.set(true);
        self.with(|owner, id| {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let decide = {
                let mut st = owner.st.borrow_mut();
                let Some(entry) = st.downloads.list.get_mut(&id) else { return };
                entry.path = Some(path.clone());
                entry.decide.take()
            };
            let Some(decide) = decide else { return };
            let file = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
            decide.call((Retained::as_ptr(&file).cast_mut(),));
            observe(owner, id, true);
            emit(owner, id, DownloadState::Running, None);
        });
    }

    fn cancel(&self) {
        self.decided.set(true);
        self.with(|owner, id| {
            let (decide, download) = {
                let mut st = owner.st.borrow_mut();
                let Some(entry) = st.downloads.list.get_mut(&id) else { return };
                (entry.decide.take(), entry.download.clone())
            };
            match decide {
                Some(decide) => decide.call((std::ptr::null_mut(),)),
                None => unsafe { download.cancel(None) },
            }
        });
    }

    fn pause(&self) {
        self.with(|owner, id| {
            let download = {
                let mut st = owner.st.borrow_mut();
                let Some(entry) = st.downloads.list.get_mut(&id) else { return };
                if entry.paused || entry.decide.is_some() {
                    return;
                }
                entry.pausing = true;
                entry.paused = true;
                entry.download.clone()
            };
            observe(owner, id, false);
            let weak = owner.this.clone();
            let block = RcBlock::new(move |data: *mut NSData| {
                let Some(owner) = weak.upgrade() else { return };
                if let Some(entry) = owner.st.borrow_mut().downloads.list.get_mut(&id) {
                    entry.resume = unsafe { data.as_ref() }.map(|d| d.retain());
                }
                emit(&owner, id, DownloadState::Running, None);
            });
            unsafe { download.cancel(Some(&block)) };
        });
    }

    fn resume(&self) {
        self.with(|owner, id| {
            let data = {
                let st = owner.st.borrow();
                let Some(entry) = st.downloads.list.get(&id) else { return };
                if !entry.paused && entry.resume.is_none() {
                    return;
                }
                entry.resume.clone()
            };
            let Some(data) = data else {
                tracing::warn!("web view: download {id} cannot resume, the server sent no resume data");
                return;
            };
            let weak = owner.this.clone();
            let block = RcBlock::new(move |download: std::ptr::NonNull<WKDownload>| {
                let Some(owner) = weak.upgrade() else { return };
                let download = unsafe { download.as_ref() };
                unsafe { download.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*owner.delegate))) };
                if let Some(entry) = owner.st.borrow_mut().downloads.list.get_mut(&id) {
                    entry.download = download.retain();
                    entry.paused = false;
                    entry.resume = None;
                    entry.observing = false;
                }
                observe(&owner, id, true);
                emit(&owner, id, DownloadState::Running, None);
            });
            unsafe { owner.web.resumeDownloadFromResumeData_completionHandler(&data, &block) };
        });
    }
}
