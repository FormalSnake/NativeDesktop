//! The tracing layer behind the flight recorder: every event that passes the filter goes into
//! the in-memory ring as structured JSON and, as one line, into the rolling log file.

use std::collections::VecDeque;
use std::fmt;
use std::io::Write;
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

pub const RING: usize = 5000;

pub(crate) type Ring = Arc<Mutex<VecDeque<Value>>>;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) type LogFile = logroller::LogRoller;
#[cfg(target_arch = "wasm32")]
pub(crate) type LogFile = std::io::Sink;

pub(crate) struct RecorderLayer {
    pub ring: Ring,
    pub file: Option<Mutex<LogFile>>,
}

struct Fields(Map<String, Value>);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0.insert(field.name().into(), format!("{value:?}").into());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().into(), value.into());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().into(), value.into());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().into(), value.into());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.0.insert(field.name().into(), value.into());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().into(), value.into());
    }
}

pub(crate) fn record(event: &Event<'_>) -> Value {
    let meta = event.metadata();
    let mut fields = Fields(Map::new());
    event.record(&mut fields);
    let mut line = Map::new();
    line.insert("t".into(), chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true).into());
    line.insert("level".into(), meta.level().as_str().into());
    line.insert("target".into(), meta.target().into());
    if let Some(Value::String(message)) = fields.0.remove("message") {
        line.insert("message".into(), message.into());
    }
    if !fields.0.is_empty() {
        line.insert("fields".into(), fields.0.into());
    }
    if let Some(thread) = std::thread::current().name() {
        line.insert("thread".into(), thread.into());
    }
    Value::Object(line)
}

pub(crate) fn push(ring: &Ring, line: Value) {
    let mut ring = ring.lock().unwrap_or_else(|e| e.into_inner());
    if ring.len() == RING {
        ring.pop_front();
    }
    ring.push_back(line);
}

impl<S: Subscriber> Layer<S> for RecorderLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let line = record(event);
        if let Some(file) = &self.file
            && let Ok(mut text) = serde_json::to_vec(&line)
        {
            text.push(b'\n');
            let _ = file.lock().unwrap_or_else(|e| e.into_inner()).write_all(&text);
        }
        push(&self.ring, line);
    }
}
