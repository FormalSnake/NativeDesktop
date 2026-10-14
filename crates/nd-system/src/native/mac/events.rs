//! App events from AppKit: activation through `NSNotificationCenter`, URL and document opens
//! through Apple event handlers (`GURL`, `odoc`). The handlers go in once the app has finished
//! launching and again a turn later, since `finishLaunching` installs AppKit's own `odoc`
//! handler over any set before it.

use std::cell::RefCell;
use std::path::PathBuf;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationDidBecomeActiveNotification, NSApplicationDidResignActiveNotification,
};
use objc2_foundation::{
    MainThreadMarker, NSAppleEventDescriptor, NSAppleEventManager, NSNotification, NSNotificationCenter, NSObject,
    NSObjectProtocol,
};

use crate::SystemEvent;

const fn four(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}

const INTERNET_EVENT_CLASS: u32 = four(b"GURL");
const GET_URL: u32 = four(b"GURL");
const CORE_EVENT_CLASS: u32 = four(b"aevt");
const OPEN_DOCUMENTS: u32 = four(b"odoc");
const DIRECT_OBJECT: u32 = four(b"----");

fn direct_object(event: &NSAppleEventDescriptor) -> Option<Retained<NSAppleEventDescriptor>> {
    // SAFETY: `paramDescriptorForKeyword:` takes an AEKeyword (a FourCharCode).
    unsafe { msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT] }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    #[unsafe(super(NSObject))]
    struct NdAppleEvents;

    unsafe impl NSObjectProtocol for NdAppleEvents {}

    impl NdAppleEvents {
        #[unsafe(method(openUrl:withReply:))]
        fn open_url(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            if let Some(url) = direct_object(event).and_then(|d| d.stringValue()) {
                super::emitter().emit(SystemEvent::OpenUrls(vec![url.to_string()]));
            }
        }

        #[unsafe(method(openDocuments:withReply:))]
        fn open_documents(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            let Some(list) = direct_object(event) else { return };
            let count = list.numberOfItems();
            // A single document may arrive as the descriptor itself rather than a list of one.
            let items: Vec<Retained<NSAppleEventDescriptor>> = if count == 0 {
                vec![list]
            } else {
                (1..=count).filter_map(|i| unsafe { msg_send![&*list, descriptorAtIndex: i] }).collect()
            };
            let paths: Vec<PathBuf> =
                items.iter().filter_map(|d| d.fileURLValue()?.path()).map(|p| PathBuf::from(p.to_string())).collect();
            if !paths.is_empty() {
                super::emitter().emit(SystemEvent::OpenFiles(paths));
            }
        }
    }
);

impl NdAppleEvents {
    fn new() -> Retained<Self> {
        unsafe { msg_send![Self::alloc(), init] }
    }
}

thread_local! {
    static HANDLERS: RefCell<Option<Retained<NdAppleEvents>>> = const { RefCell::new(None) };
    static OBSERVERS: RefCell<Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>> = const { RefCell::new(Vec::new()) };
}

fn install_apple_events() {
    let handlers = HANDLERS.with_borrow_mut(|h| h.get_or_insert_with(NdAppleEvents::new).clone());
    let manager = NSAppleEventManager::sharedAppleEventManager();
    let handler: &AnyObject = &handlers;
    // SAFETY: the handler is retained in `HANDLERS` for the life of the process and implements
    // both selectors with the signature the manager calls.
    unsafe {
        let _: () = msg_send![&*manager, setEventHandler: handler, andSelector: sel!(openUrl:withReply:), forEventClass: INTERNET_EVENT_CLASS, andEventID: GET_URL];
        let _: () = msg_send![&*manager, setEventHandler: handler, andSelector: sel!(openDocuments:withReply:), forEventClass: CORE_EVENT_CLASS, andEventID: OPEN_DOCUMENTS];
    }
}

pub fn install(mtm: MainThreadMarker) {
    install_apple_events();
    dispatch2::DispatchQueue::main().exec_async(install_apple_events);
    let center = NSNotificationCenter::defaultCenter();
    // SAFETY: framework constants; the blocks are retained by the center until the observers
    // (kept in `OBSERVERS`) are removed, which they never are.
    unsafe {
        for (name, event) in [
            (NSApplicationDidBecomeActiveNotification, SystemEvent::Activated),
            (NSApplicationDidResignActiveNotification, SystemEvent::Deactivated),
        ] {
            let block = RcBlock::new(move |_: NonNull<NSNotification>| super::emitter().emit(event.clone()));
            let observer = center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block);
            OBSERVERS.with_borrow_mut(|o| o.push(observer));
        }
    }
    if !NSApplication::sharedApplication(mtm).isActive() {
        super::emitter().emit(SystemEvent::Deactivated);
    }
}
