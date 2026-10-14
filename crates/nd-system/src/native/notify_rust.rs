//! Notifications through `notify-rust`: freedesktop notifications on Linux, toasts on Windows.
//! Each one shown keeps a thread waiting for its action or close. On Linux a body click is the
//! spec's `default` action, so every notification carries one; Windows reports it unasked.

use crate::backend::{Emitter, Reply};
use crate::{AppIdentity, Error, Notification, NotificationId, SystemEvent};

pub fn show(app: &AppIdentity, events: Emitter, id: NotificationId, n: Notification, reply: Reply<()>) {
    let app = app.clone();
    std::thread::spawn(move || {
        let mut native = notify_rust::Notification::new();
        native.appname(&app.display_name).summary(&n.title);
        #[cfg(windows)]
        {
            native.app_id(&app.id);
            if let Some(subtitle) = &n.subtitle {
                native.subtitle(subtitle);
            }
            if let Some(body) = &n.body {
                native.body(body);
            }
        }
        #[cfg(target_os = "linux")]
        {
            use notify_rust::Hint;
            native.hint(Hint::DesktopEntry(app.id.clone()));
            // The spec has no subtitle; it leads the body instead.
            let body = [n.subtitle.as_deref(), n.body.as_deref()].into_iter().flatten().collect::<Vec<_>>().join("\n");
            if !body.is_empty() {
                native.body(&body);
            }
            native.action("default", "");
            if n.silent {
                native.hint(Hint::SuppressSound(true));
            }
        }
        for a in &n.actions {
            native.action(&a.id, &a.title);
        }
        let handle = match native.show() {
            Ok(handle) => handle,
            Err(e) => return reply.err(Error::platform(e)),
        };
        reply.ok(());
        handle.wait_for_action(|action| {
            let action = match action {
                "__closed" => return,
                "default" => None,
                other => Some(other.to_owned()),
            };
            events.emit(SystemEvent::NotificationClicked { id, action, data: None });
        });
    });
}
