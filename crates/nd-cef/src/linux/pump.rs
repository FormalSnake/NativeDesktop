//! Chromium on GPUI's main loop (`external_message_pump`): one foreground task runs
//! `do_message_loop_work` when Chromium asks for it (`on_schedule_message_pump_work`, from any
//! thread) or when its fallback interval passes, then redraws the windows whose pages painted.
//! The task starts with the first page shown in a window, since only a window hands over
//! GPUI's app.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use cef::do_message_loop_work;
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::future::{Either, select};
use nd_backend_gpui::GpuiBackend;
use nd_gpui::gpui::{AsyncApp, Task};

/// The longest Chromium waits for its work without a scheduled call, while pages are open.
const FALLBACK: Duration = Duration::from_millis(33);
/// The same, with no page open.
const IDLE: Duration = Duration::from_secs(1);
/// How long shutdown lets closing browsers finish.
const SHUTDOWN: Duration = Duration::from_secs(3);

pub(crate) enum Msg {
    /// Chromium wants `do_message_loop_work` after this many milliseconds.
    Work(i64),
    /// Work for the main thread from another one.
    Run(Box<dyn FnOnce() + Send>),
    /// [`later`] queued work.
    Later,
    /// Pages need their windows redrawn (focus moved).
    Redraw,
}

static SENDER: OnceLock<UnboundedSender<Msg>> = OnceLock::new();

thread_local! {
    static RECEIVER: RefCell<Option<UnboundedReceiver<Msg>>> = const { RefCell::new(None) };
    static TASK: RefCell<Option<Task<()>>> = const { RefCell::new(None) };
    static LATER: RefCell<VecDeque<Box<dyn FnOnce()>>> = RefCell::default();
    static PUMPING: Cell<bool> = const { Cell::new(false) };
}

fn sender() -> &'static UnboundedSender<Msg> {
    SENDER.get_or_init(|| {
        let (tx, rx) = unbounded();
        RECEIVER.with(|r| *r.borrow_mut() = Some(rx));
        tx
    })
}

/// Before `CefInitialize`: the channel exists on the main thread.
pub(crate) fn prepare() {
    sender();
}

/// After `CefInitialize`. The loop itself starts with [`attach_backend`].
pub(crate) fn start() {}

pub(crate) fn send(msg: Msg) {
    let _ = sender().unbounded_send(msg);
}

/// `on_schedule_message_pump_work`: any thread.
pub(crate) fn schedule(delay_ms: i64) {
    send(Msg::Work(delay_ms));
}

/// Runs `f` on a later turn of the main loop.
pub(crate) fn later(f: Box<dyn FnOnce()>) {
    LATER.with(|l| l.borrow_mut().push_back(f));
    send(Msg::Later);
}

/// Starts the loop on the GPUI executor of the first window that shows a page, once, and
/// shuts Chromium down when the app quits.
pub(crate) fn attach_backend(backend: &GpuiBackend) {
    if TASK.with(|t| t.borrow().is_some()) {
        return;
    }
    let Some(app) = backend.async_app() else { return };
    let Some(rx) = RECEIVER.with(|r| r.borrow_mut().take()) else { return };
    let task = app.spawn(async move |cx| {
        cx.update(|cx| {
            cx.on_app_quit(|_| {
                shutdown();
                async {}
            })
            .detach();
        });
        run_loop(rx, cx).await;
    });
    TASK.with(|t| *t.borrow_mut() = Some(task));
}

async fn run_loop(mut rx: UnboundedReceiver<Msg>, cx: &mut AsyncApp) {
    let mut deadline = Instant::now();
    loop {
        let wait = deadline.saturating_duration_since(Instant::now());
        let timer = cx.background_executor().timer(wait);
        let msg = match select(rx.next(), timer).await {
            Either::Left((None, _)) => return,
            Either::Left((Some(msg), _)) => Some(msg),
            Either::Right(_) => None,
        };
        let mut due = msg.is_none();
        let mut handle = |msg: Msg, deadline: &mut Instant| match msg {
            Msg::Work(delay) if delay <= 0 => due = true,
            Msg::Work(delay) => {
                *deadline = (*deadline).min(Instant::now() + Duration::from_millis(delay as u64));
            }
            Msg::Run(f) => f(),
            Msg::Later => {
                while let Some(f) = LATER.with(|l| l.borrow_mut().pop_front()) {
                    f();
                }
            }
            Msg::Redraw => {}
        };
        if let Some(msg) = msg {
            handle(msg, &mut deadline);
        }
        while let Ok(msg) = rx.try_recv() {
            handle(msg, &mut deadline);
        }
        if due || Instant::now() >= deadline {
            run();
            let fallback = if crate::page::live_count() > 0 { FALLBACK } else { IDLE };
            deadline = Instant::now() + fallback;
        }
        super::view::flush(cx);
    }
}

/// One `do_message_loop_work`, never nested: a call made while Chromium is inside one runs once
/// it returns.
pub(crate) fn run() {
    if PUMPING.with(|p| p.replace(true)) {
        return;
    }
    do_message_loop_work();
    PUMPING.with(|p| p.set(false));
}

/// Closes every browser, lets Chromium finish closing them, then shuts it down.
fn shutdown() {
    crate::page::close_all();
    let deadline = Instant::now() + SHUTDOWN;
    while crate::page::browser_count() > 0 && Instant::now() < deadline {
        do_message_loop_work();
        std::thread::sleep(Duration::from_millis(5));
    }
    cef::shutdown();
}
