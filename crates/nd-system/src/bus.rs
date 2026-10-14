//! Fans OS events out to every subscriber. Open requests that arrive before anyone subscribed
//! (the URL or document a launch was for) are held and replayed to the first subscriber, so an
//! app that subscribes after building its windows still sees them; the zig branch dropped
//! those.

use std::collections::{HashMap, VecDeque};
use std::future::poll_fn;
use std::pin::Pin;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use futures_channel::mpsc;
use futures_core::Stream;

use crate::{NotificationId, SystemEvent};

/// Click payloads kept per process; the oldest goes first.
const NOTIFICATION_DATA_CAP: usize = 128;

#[derive(Default)]
struct State {
    subscribers: Vec<mpsc::UnboundedSender<SystemEvent>>,
    held: Vec<SystemEvent>,
    data: HashMap<NotificationId, serde_json::Value>,
    data_order: VecDeque<NotificationId>,
}

pub(crate) struct Bus {
    state: Mutex<State>,
    active: AtomicBool,
}

impl Bus {
    pub fn new() -> Self {
        Self { state: Mutex::default(), active: AtomicBool::new(true) }
    }

    pub fn subscribe(&self) -> Events {
        let (tx, rx) = mpsc::unbounded();
        let mut state = self.state.lock().expect("event bus");
        for event in state.held.drain(..) {
            let _ = tx.unbounded_send(event);
        }
        state.subscribers.push(tx);
        Events(rx)
    }

    pub fn emit(&self, mut event: SystemEvent) {
        match &event {
            SystemEvent::Activated => self.active.store(true, Ordering::Relaxed),
            SystemEvent::Deactivated => self.active.store(false, Ordering::Relaxed),
            _ => {}
        }
        let mut state = self.state.lock().expect("event bus");
        if let SystemEvent::NotificationClicked { id, data, .. } = &mut event
            && data.is_none()
        {
            *data = state.data.get(id).cloned();
        }
        state.subscribers.retain(|s| !s.is_closed());
        if state.subscribers.is_empty() {
            if matches!(event, SystemEvent::OpenUrls(_) | SystemEvent::OpenFiles(_)) {
                state.held.push(event);
            }
            return;
        }
        for s in &state.subscribers {
            let _ = s.unbounded_send(event.clone());
        }
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    pub fn keep_data(&self, id: NotificationId, data: serde_json::Value) {
        let mut state = self.state.lock().expect("event bus");
        if state.data.insert(id.clone(), data).is_none() {
            state.data_order.push_back(id);
        }
        while state.data_order.len() > NOTIFICATION_DATA_CAP {
            if let Some(old) = state.data_order.pop_front() {
                state.data.remove(&old);
            }
        }
    }
}

/// The app's OS events, from [`crate::System::events`]. A `Stream`; [`Events::next`] for
/// `async` code without a stream extension trait. Dropping it unsubscribes.
pub struct Events(mpsc::UnboundedReceiver<SystemEvent>);

impl Events {
    pub async fn next(&mut self) -> Option<SystemEvent> {
        poll_fn(|cx| Pin::new(&mut self.0).poll_next(cx)).await
    }

    /// The next event already queued, without waiting.
    pub fn try_next(&mut self) -> Option<SystemEvent> {
        self.0.try_recv().ok()
    }
}

impl Stream for Events {
    type Item = SystemEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<SystemEvent>> {
        Pin::new(&mut self.0).poll_next(cx)
    }
}
