//! Notifications. A bundled app (one with a bundle identifier) uses UserNotifications, asking
//! for permission on the first notification. An unbundled binary (`cargo run`, tests) cannot:
//! `UNUserNotificationCenter` raises for a process without a bundle, so it falls back to the
//! deprecated `NSUserNotificationCenter`, which shows the first action as its button, where
//! the OS still serves one (recent macOS does not: the call then fails with a clear error).

#![allow(deprecated)]

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject};
use objc2::{AnyThread, ClassType, define_class, msg_send};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSBundle, NSError, NSObject, NSObjectProtocol, NSSet, NSString, NSUserNotification,
    NSUserNotificationActivationType, NSUserNotificationCenter, NSUserNotificationCenterDelegate,
    NSUserNotificationDefaultSoundName,
};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAction,
    UNNotificationActionOptions, UNNotificationCategory, UNNotificationCategoryOptions,
    UNNotificationDefaultActionIdentifier, UNNotificationDismissActionIdentifier, UNNotificationPresentationOptions,
    UNNotificationRequest, UNNotificationResponse, UNNotificationSound, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};

use crate::backend::Reply;
use crate::{Error, Notification, NotificationId, SystemEvent};

fn clicked(id: &NSString, action: Option<String>) {
    let id = NotificationId(id.to_string());
    super::emitter().emit(SystemEvent::NotificationClicked { id, action, data: None });
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this type does not implement Drop.
    // The centers call their delegates on queues of their choosing, so it is not main-thread only.
    #[unsafe(super(NSObject))]
    struct NdNotificationDelegate;

    unsafe impl NSObjectProtocol for NdNotificationDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for NdNotificationDelegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            handler: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            // Shown while the app is frontmost too, as on every other platform.
            handler.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            handler: &DynBlock<dyn Fn()>,
        ) {
            let action = response.actionIdentifier();
            // SAFETY: framework constants, valid for the life of the process.
            let (default, dismiss) =
                unsafe { (UNNotificationDefaultActionIdentifier, UNNotificationDismissActionIdentifier) };
            if *action != *dismiss {
                let action = (*action != *default).then(|| action.to_string());
                clicked(&response.notification().request().identifier(), action);
            }
            handler.call(());
        }
    }

    unsafe impl NSUserNotificationCenterDelegate for NdNotificationDelegate {
        #[unsafe(method(userNotificationCenter:didActivateNotification:))]
        fn did_activate(&self, _center: &NSUserNotificationCenter, notification: &NSUserNotification) {
            let Some(id) = notification.identifier() else { return };
            let action = match notification.activationType() {
                NSUserNotificationActivationType::ContentsClicked => None,
                NSUserNotificationActivationType::ActionButtonClicked => {
                    ACTION_BUTTONS.lock().expect("action buttons").get(&id.to_string()).cloned()
                }
                _ => return,
            };
            clicked(&id, action);
        }

        #[unsafe(method(userNotificationCenter:shouldPresentNotification:))]
        fn should_present(&self, _center: &NSUserNotificationCenter, _notification: &NSUserNotification) -> bool {
            true
        }
    }
);

impl NdNotificationDelegate {
    fn new() -> Retained<Self> {
        unsafe { msg_send![Self::alloc(), init] }
    }
}

thread_local! {
    /// The centers hold their delegate weakly.
    static DELEGATE: RefCell<Option<Retained<NdNotificationDelegate>>> = const { RefCell::new(None) };
    static AUTHORIZED: Cell<bool> = const { Cell::new(false) };
    /// Category identifier to its action ids, for every action set shown so far.
    static CATEGORIES: RefCell<BTreeMap<String, Vec<(String, String)>>> = const { RefCell::new(BTreeMap::new()) };
}

/// Legacy notification id to the action its button stands for. The delegate reads it on the
/// center's queue.
static ACTION_BUTTONS: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

fn delegate() -> Retained<NdNotificationDelegate> {
    DELEGATE.with_borrow_mut(|d| d.get_or_insert_with(NdNotificationDelegate::new).clone())
}

fn bundled() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some()
}

pub fn show(_mtm: MainThreadMarker, id: NotificationId, n: Notification, reply: Reply<()>) {
    if bundled() { show_un(id, n, reply) } else { show_legacy(id, n, reply) }
}

fn category(center: &UNUserNotificationCenter, actions: &[(String, String)]) -> Option<String> {
    if actions.is_empty() {
        return None;
    }
    let key = format!("nd-actions-{}", actions.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>().join("|"));
    let added = CATEGORIES.with_borrow_mut(|c| c.insert(key.clone(), actions.to_vec()).is_none());
    if added {
        let all: Vec<Retained<UNNotificationCategory>> = CATEGORIES.with_borrow(|c| {
            c.iter()
                .map(|(key, actions)| {
                    let actions: Vec<Retained<UNNotificationAction>> = actions
                        .iter()
                        .map(|(id, title)| {
                            UNNotificationAction::actionWithIdentifier_title_options(
                                &NSString::from_str(id),
                                &NSString::from_str(title),
                                UNNotificationActionOptions::Foreground,
                            )
                        })
                        .collect();
                    UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
                        &NSString::from_str(key),
                        &NSArray::from_retained_slice(&actions),
                        &NSArray::new(),
                        UNNotificationCategoryOptions::empty(),
                    )
                })
                .collect()
        });
        center.setNotificationCategories(&NSSet::from_retained_slice(&all));
    }
    Some(key)
}

fn show_un(id: NotificationId, n: Notification, reply: Reply<()>) {
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let delegate = delegate();
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&n.title));
    if let Some(subtitle) = &n.subtitle {
        content.setSubtitle(&NSString::from_str(subtitle));
    }
    if let Some(body) = &n.body {
        content.setBody(&NSString::from_str(body));
    }
    if !n.silent {
        content.setSound(Some(&UNNotificationSound::defaultSound()));
    }
    let actions: Vec<(String, String)> = n.actions.iter().map(|a| (a.id.clone(), a.title.clone())).collect();
    if let Some(category) = category(&center, &actions) {
        content.setCategoryIdentifier(&NSString::from_str(&category));
    }
    let request =
        UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&id.0), &content, None);
    let reply = Arc::new(Mutex::new(Some(reply)));
    let answer = move |result: crate::Result<()>| {
        if let Some(reply) = reply.lock().expect("notify reply").take() {
            reply.send(result);
        }
    };
    let added = {
        let answer = answer.clone();
        RcBlock::new(move |error: *mut NSError| {
            // SAFETY: the framework passes null or a valid error for the duration of the call.
            answer(match unsafe { error.as_ref() } {
                None => Ok(()),
                Some(e) => Err(Error::Denied(e.localizedDescription().to_string())),
            })
        })
    };
    if AUTHORIZED.get() {
        center.addNotificationRequest_withCompletionHandler(&request, Some(&added));
        return;
    }
    // The first notification asks for permission and is delivered once that is answered.
    AUTHORIZED.set(true);
    let pending = Mutex::new(Some((center.clone(), request, added)));
    let asked = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
        let Some((center, request, added)) = pending.lock().expect("notify pending").take() else { return };
        if granted.as_bool() {
            center.addNotificationRequest_withCompletionHandler(&request, Some(&added));
        } else {
            answer(Err(Error::Denied("notifications are turned off for this app".into())));
        }
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &asked,
    );
}

fn show_legacy(id: NotificationId, n: Notification, reply: Reply<()>) {
    // Recent macOS answers nil here for a process without a bundle.
    let center: Option<Retained<NSUserNotificationCenter>> =
        unsafe { msg_send![NSUserNotificationCenter::class(), defaultUserNotificationCenter] };
    let Some(center) = center else {
        return reply.err(Error::Platform("notifications need a bundled app (no bundle identifier)".into()));
    };
    let delegate = delegate();
    // SAFETY: the delegate is retained in `DELEGATE` for the life of the process.
    unsafe { center.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
    let native = NSUserNotification::new();
    native.setIdentifier(Some(&NSString::from_str(&id.0)));
    native.setTitle(Some(&NSString::from_str(&n.title)));
    native.setSubtitle(n.subtitle.as_deref().map(NSString::from_str).as_deref());
    native.setInformativeText(n.body.as_deref().map(NSString::from_str).as_deref());
    if !n.silent {
        // SAFETY: a framework constant.
        native.setSoundName(Some(unsafe { NSUserNotificationDefaultSoundName }));
    }
    match n.actions.first() {
        Some(action) => {
            native.setHasActionButton(true);
            native.setActionButtonTitle(&NSString::from_str(&action.title));
            ACTION_BUTTONS.lock().expect("action buttons").insert(id.0.clone(), action.id.clone());
        }
        None => native.setHasActionButton(false),
    }
    center.deliverNotification(&native);
    reply.ok(());
}
