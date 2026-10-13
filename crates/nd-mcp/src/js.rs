//! `execute`: agent-written JavaScript in QuickJS with `app`, `state` and `console` bound.
//!
//! The JS API mirrors `nd-test`: `app.getByRole(..).click()`, `app.locator(selector)`, `app.snapshot()`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use rquickjs::{CatchResultExt, Context, Function, Runtime};
use serde_json::{Value, json};

use crate::ops;
use crate::session::Bridge;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

const PRELUDE: &str = include_str!("prelude.js");

#[derive(Debug, PartialEq)]
pub struct Executed {
    pub result: Value,
    pub logs: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub struct ExecError {
    pub message: String,
    pub logs: Vec<String>,
}

enum Command {
    Run {
        code: String,
        timeout: Duration,
    },
    /// Drop the context, and with it `state`.
    Reset,
}

type Reply = mpsc::Sender<Result<Executed, ExecError>>;

/// Handle to the JS thread. QuickJS values never leave it.
#[derive(Clone)]
pub struct JsEngine {
    tx: mpsc::Sender<(Command, Reply)>,
}

struct Vm {
    // Kept alive for the context.
    _runtime: Runtime,
    context: Context,
    logs: Arc<Mutex<Vec<String>>>,
    deadline: Arc<Mutex<Option<Instant>>>,
    timed_out: Arc<AtomicBool>,
}

impl JsEngine {
    pub fn spawn(session: Bridge) -> Self {
        let (tx, rx) = mpsc::channel::<(Command, Reply)>();
        thread::Builder::new()
            .name("nd-mcp-js".into())
            .spawn(move || {
                let mut vm: Option<Vm> = None;
                while let Ok((command, reply)) = rx.recv() {
                    let answer = match command {
                        Command::Reset => {
                            vm = None;
                            Ok(Executed { result: Value::Null, logs: Vec::new() })
                        }
                        Command::Run { code, timeout } => {
                            let machine = match &mut vm {
                                Some(machine) => Ok(machine),
                                None => build(&session).map(|built| vm.insert(built)),
                            };
                            match machine {
                                Ok(machine) => run(machine, &code, timeout),
                                Err(message) => Err(ExecError { message, logs: Vec::new() }),
                            }
                        }
                    };
                    let _ = reply.send(answer);
                }
            })
            .expect("spawn js thread");
        Self { tx }
    }

    fn send(&self, command: Command) -> Result<Executed, ExecError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        let gone = || ExecError { message: "the JS thread is gone".into(), logs: Vec::new() };
        self.tx.send((command, reply_tx)).map_err(|_| gone())?;
        reply_rx.recv().map_err(|_| gone())?
    }

    pub fn execute(&self, code: &str, timeout: Duration) -> Result<Executed, ExecError> {
        self.send(Command::Run { code: code.to_string(), timeout })
    }

    pub fn reset(&self) {
        let _ = self.send(Command::Reset);
    }
}

fn envelope<T: serde::Serialize, E: std::fmt::Display>(result: Result<T, E>) -> String {
    match result {
        Ok(value) => json!({ "ok": value }).to_string(),
        Err(error) => json!({ "error": error.to_string() }).to_string(),
    }
}

fn build(session: &Bridge) -> Result<Vm, String> {
    let runtime = Runtime::new().map_err(|e| e.to_string())?;
    runtime.set_memory_limit(128 * 1024 * 1024);
    let deadline: Arc<Mutex<Option<Instant>>> = Arc::default();
    let timed_out = Arc::new(AtomicBool::new(false));
    {
        let (deadline, timed_out) = (deadline.clone(), timed_out.clone());
        runtime.set_interrupt_handler(Some(Box::new(move || {
            let expired = deadline.lock().is_ok_and(|d| d.is_some_and(|at| Instant::now() > at));
            if expired {
                timed_out.store(true, Ordering::Relaxed);
            }
            expired
        })));
    }
    let context = Context::full(&runtime).map_err(|e| e.to_string())?;
    let logs: Arc<Mutex<Vec<String>>> = Arc::default();

    context
        .with(|ctx| -> rquickjs::Result<()> {
            let globals = ctx.globals();
            let log_sink = logs.clone();
            globals.set(
                "__nd_log",
                Function::new(ctx.clone(), move |line: String| {
                    if let Ok(mut logs) = log_sink.lock() {
                        logs.push(line);
                    }
                })?,
            )?;
            let bridge = session.clone();
            globals.set(
                "__nd",
                Function::new(ctx.clone(), move |op: String| match serde_json::from_str::<Value>(&op) {
                    Ok(op) => envelope(bridge.run(move |session| ops::dispatch(session, &op))),
                    Err(error) => envelope::<Value, _>(Err(error)),
                })?,
            )?;
            ctx.eval::<(), _>(PRELUDE)?;
            Ok(())
        })
        .map_err(|e| format!("setting up the JS context: {e}"))?;

    Ok(Vm { _runtime: runtime, context, logs, deadline, timed_out })
}

fn run(vm: &mut Vm, code: &str, timeout: Duration) -> Result<Executed, ExecError> {
    if let Ok(mut logs) = vm.logs.lock() {
        logs.clear();
    }
    vm.timed_out.store(false, Ordering::Relaxed);
    if let Ok(mut deadline) = vm.deadline.lock() {
        *deadline = Some(Instant::now() + timeout);
    }
    let outcome: Result<String, String> = vm.context.with(|ctx| {
        let run: Function = ctx.globals().get("__nd_run").map_err(|e| e.to_string())?;
        run.call::<_, String>((code,)).catch(&ctx).map_err(|e| e.to_string())
    });
    if let Ok(mut deadline) = vm.deadline.lock() {
        *deadline = None;
    }
    let logs = vm.logs.lock().map(|l| l.clone()).unwrap_or_default();
    match outcome {
        Ok(json) => Ok(Executed { result: serde_json::from_str(&json).unwrap_or(Value::Null), logs }),
        Err(_) if vm.timed_out.load(Ordering::Relaxed) => {
            Err(ExecError { message: format!("execute timed out after {}ms", timeout.as_millis()), logs })
        }
        Err(message) => Err(ExecError { message, logs }),
    }
}
