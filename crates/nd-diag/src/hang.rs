//! Main-thread hang watchdog (ARCHITECTURE.md 9.4). A watchdog thread keeps one ping queued on
//! the main thread; a ping left unanswered past the threshold writes a `-hang` bundle, with a
//! minidump holding every thread's stack when the crash helper runs.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::Shared;
use crate::bundle::Kind;

/// Runs a closure on the main thread, from any thread. The host's main queue.
pub type PostToMain = Box<dyn Fn(Box<dyn FnOnce() + Send>) + Send + Sync>;

/// Stops the watchdog thread when dropped.
pub struct Watchdog {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

pub(crate) fn start(shared: Weak<Shared>, post: PostToMain, threshold: Duration) -> Watchdog {
    let stop = Arc::new(AtomicBool::new(false));
    let answered = Arc::new(AtomicU64::new(0));
    let tick = (threshold / 5).clamp(Duration::from_millis(10), Duration::from_secs(1));
    let thread = {
        let stop = stop.clone();
        std::thread::Builder::new()
            .name("nd-hang-watchdog".into())
            .spawn(move || {
                let mut sent = 0u64;
                let mut sent_at = Instant::now();
                let mut reported = false;
                while !stop.load(Ordering::SeqCst) {
                    if answered.load(Ordering::SeqCst) == sent {
                        if reported {
                            let blocked = sent_at.elapsed();
                            tracing::warn!(blocked_ms = blocked.as_millis() as u64, "main thread answered again");
                            reported = false;
                        }
                        sent += 1;
                        sent_at = Instant::now();
                        let (answered, seq) = (answered.clone(), sent);
                        post(Box::new(move || answered.store(seq, Ordering::SeqCst)));
                    } else if !reported && sent_at.elapsed() >= threshold {
                        reported = true;
                        let Some(shared) = shared.upgrade() else { return };
                        let blocked = sent_at.elapsed();
                        tracing::error!(blocked_ms = blocked.as_millis() as u64, "main thread hang");
                        let detail = json!({
                            "thread": "main",
                            "blocked_ms": blocked.as_millis() as u64,
                            "threshold_ms": threshold.as_millis() as u64,
                        });
                        let _ = shared.write_incident(
                            Kind::Hang,
                            "hang.json",
                            &serde_json::to_vec_pretty(&detail).unwrap_or_default(),
                        );
                    }
                    std::thread::park_timeout(tick);
                }
            })
            .expect("spawn hang watchdog")
    };
    Watchdog { stop, thread: Some(thread) }
}
