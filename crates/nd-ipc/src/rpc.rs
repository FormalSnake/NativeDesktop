//! JSON-RPC 2.0, one message per line, with typed contracts.
//!
//! A [`Method`] names a call and fixes its params and result types, so the server handler and
//! every client call site agree at compile time. The [`Client`] owns a background task that
//! connects, reconnects on a [`Ladder`] and exposes its [`Status`]; calls made while it is not
//! connected fail fast with [`CallError::NotConnected`] rather than queueing.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use interprocess::local_socket::{ListenerOptions, tokio::prelude::*};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::Endpoint;
use crate::framing::{BoxStream, pump};
use crate::ladder::{Ladder, LadderConfig};

pub mod codes {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
}

/// A request/response contract.
pub trait Method {
    const NAME: &'static str;
    type Params: Serialize + DeserializeOwned + Send + 'static;
    type Output: Serialize + DeserializeOwned + Send + 'static;
}

/// A one-way message (no id, no reply): server pushes and client hints.
pub trait Notification {
    const NAME: &'static str;
    type Params: Serialize + DeserializeOwned + Send + 'static;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[error("rpc error {code}: {message}")]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), data: None }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(codes::INTERNAL_ERROR, message)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CallError {
    #[error("not connected")]
    NotConnected,
    #[error("connection lost before the reply")]
    Disconnected,
    #[error("no reply within {0:?}")]
    Timeout(Duration),
    #[error(transparent)]
    Rpc(#[from] RpcError),
    #[error("params or result did not match the contract: {0}")]
    Codec(#[from] serde_json::Error),
    #[error("client closed")]
    Closed,
}

#[derive(Serialize, Deserialize)]
struct Message {
    jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

impl Message {
    fn blank() -> Self {
        Self { jsonrpc: "2.0".into(), id: None, method: None, params: None, result: None, error: None }
    }

    fn line(&self) -> String {
        serde_json::to_string(self).expect("a Message always serializes")
    }
}

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
type Handler = Arc<dyn Fn(Value) -> BoxFuture<Result<Value, RpcError>> + Send + Sync>;
type Peers = Arc<Mutex<Vec<mpsc::UnboundedSender<String>>>>;

fn locked<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ---------------------------------------------------------------- server

/// Handlers keyed by method name.
#[derive(Default)]
pub struct Server {
    handlers: HashMap<&'static str, Handler>,
}

impl Server {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers the handler for `M`. Bad params are answered with `-32602` before the
    /// handler runs.
    pub fn method<M, F, Fut>(mut self, handler: F) -> Self
    where
        M: Method,
        F: Fn(M::Params) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<M::Output, RpcError>> + Send + 'static,
    {
        let handler = Arc::new(handler);
        let erased: Handler = Arc::new(move |params: Value| {
            let handler = Arc::clone(&handler);
            Box::pin(async move {
                let params = serde_json::from_value::<M::Params>(params)
                    .map_err(|e| RpcError::new(codes::INVALID_PARAMS, e.to_string()))?;
                let output = handler(params).await?;
                serde_json::to_value(output).map_err(|e| RpcError::internal(e.to_string()))
            })
        });
        self.handlers.insert(M::NAME, erased);
        self
    }

    /// Accepts connections on `endpoint` until the returned handle is dropped. A stale socket
    /// file is replaced, so only one server per endpoint should run (use the instance lock).
    pub fn serve(self, endpoint: &Endpoint) -> std::io::Result<ServerHandle> {
        if let Some(dir) = endpoint.socket_path().parent() {
            std::fs::create_dir_all(dir)?;
        }
        if !cfg!(windows) {
            match std::fs::remove_file(endpoint.socket_path()) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        let listener = ListenerOptions::new().name(endpoint.name()?).create_tokio()?;
        let handlers = Arc::new(self.handlers);
        let peers: Peers = Arc::default();
        let accept_peers = Arc::clone(&peers);
        let task = tokio::spawn(async move {
            // Connections live in the set, so aborting this task closes them too.
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok(stream) = accepted else { continue };
                        connections.spawn(serve_connection(stream, Arc::clone(&handlers), Arc::clone(&accept_peers)));
                    }
                    Some(_) = connections.join_next() => {}
                }
            }
        });
        Ok(ServerHandle { task, peers, socket: endpoint.socket_path().to_path_buf() })
    }

    /// Serves one already-open stream until it closes. For tests and non-socket transports.
    pub async fn serve_stream<S>(self, stream: S)
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        serve_connection(stream, Arc::new(self.handlers), Arc::default()).await;
    }
}

pub struct ServerHandle {
    task: JoinHandle<()>,
    peers: Peers,
    socket: std::path::PathBuf,
}

impl ServerHandle {
    /// Pushes `N` to every connected client.
    pub fn notify<N: Notification>(&self, params: &N::Params) -> Result<(), serde_json::Error> {
        let line = notification_line(N::NAME, params)?;
        locked(&self.peers).retain(|peer| peer.send(line.clone()).is_ok());
        Ok(())
    }

    pub fn client_count(&self) -> usize {
        locked(&self.peers).retain(|peer| !peer.is_closed());
        locked(&self.peers).len()
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.task.abort();
        if !cfg!(windows) {
            let _ = std::fs::remove_file(&self.socket);
        }
    }
}

fn notification_line<P: Serialize>(method: &str, params: &P) -> Result<String, serde_json::Error> {
    let mut msg = Message::blank();
    msg.method = Some(method.into());
    msg.params = Some(serde_json::to_value(params)?);
    Ok(msg.line())
}

async fn serve_connection<S>(stream: S, handlers: Arc<HashMap<&'static str, Handler>>, peers: Peers)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (tx, rx) = mpsc::unbounded_channel::<String>();
    locked(&peers).push(tx.clone());
    let reply_tx = tx;
    let result = pump(stream, rx, |line| {
        let Some(request) = parse_request(line, &reply_tx) else { return };
        let (id, method, params) = request;
        let Some(handler) = handlers.get(method.as_str()).cloned() else {
            if let Some(id) = id {
                send_reply(&reply_tx, id, Err(RpcError::new(codes::METHOD_NOT_FOUND, format!("no method {method}"))));
            }
            return;
        };
        let reply_tx = reply_tx.clone();
        tokio::spawn(async move {
            let outcome = handler(params).await;
            if let Some(id) = id {
                send_reply(&reply_tx, id, outcome);
            }
        });
    })
    .await;
    if let Err(e) = result {
        tracing::debug!("rpc connection ended: {e}");
    }
}

/// `(id, method, params)` of a request or notification; answers malformed input itself.
fn parse_request(line: &str, reply: &mpsc::UnboundedSender<String>) -> Option<(Option<u64>, String, Value)> {
    let msg: Message = match serde_json::from_str(line) {
        Ok(msg) => msg,
        Err(e) => {
            let mut out = Message::blank();
            out.error = Some(RpcError::new(codes::PARSE_ERROR, e.to_string()));
            let _ = reply.send(out.line());
            return None;
        }
    };
    let Some(method) = msg.method else {
        // A response on the server side is not part of this protocol.
        if let Some(id) = msg.id {
            send_reply(reply, id, Err(RpcError::new(codes::INVALID_REQUEST, "expected a request")));
        }
        return None;
    };
    Some((msg.id, method, msg.params.unwrap_or(Value::Null)))
}

fn send_reply(tx: &mpsc::UnboundedSender<String>, id: u64, outcome: Result<Value, RpcError>) {
    let mut out = Message::blank();
    out.id = Some(id);
    match outcome {
        Ok(value) => out.result = Some(value),
        Err(e) => out.error = Some(e),
    }
    let _ = tx.send(out.line());
}

// ---------------------------------------------------------------- client

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Connecting,
    Connected,
    /// Between attempts: this is attempt `attempt`'s failure, retrying after `delay`.
    Waiting {
        attempt: u32,
        delay: Duration,
    },
    Closed,
}

#[derive(Clone, Copy, Debug)]
pub struct ClientOptions {
    pub ladder: LadderConfig,
    pub call_timeout: Duration,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self { ladder: LadderConfig::default(), call_timeout: Duration::from_secs(10) }
    }
}

/// A notification from the server, decoded on demand with [`Event::decode`].
#[derive(Clone, Debug)]
pub struct Event {
    pub method: String,
    pub params: Value,
}

impl Event {
    /// `Some` when this event is an `N` and its params match the contract.
    pub fn decode<N: Notification>(&self) -> Option<N::Params> {
        (self.method == N::NAME).then(|| serde_json::from_value(self.params.clone()).ok()).flatten()
    }
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>>;

struct Live {
    tx: mpsc::UnboundedSender<String>,
    pending: Pending,
}

pub type Connector = Box<dyn Fn() -> BoxFuture<std::io::Result<BoxStream>> + Send + Sync>;

struct Shared {
    live: Mutex<Option<Live>>,
    next_id: AtomicU64,
    status: watch::Sender<Status>,
    events: broadcast::Sender<Event>,
    close: watch::Sender<bool>,
    options: ClientOptions,
}

pub struct Client {
    shared: Arc<Shared>,
    task: JoinHandle<()>,
}

impl Client {
    /// Connects to `endpoint` and keeps reconnecting. Returns immediately; watch
    /// [`Client::status`] or use [`Client::wait_connected`]. Must run inside a tokio runtime.
    pub fn connect(endpoint: &Endpoint, options: ClientOptions) -> Self {
        let endpoint = endpoint.clone();
        Self::with_connector(
            Box::new(move || {
                let endpoint = endpoint.clone();
                Box::pin(async move { Ok(Box::new(crate::connect(&endpoint).await?) as BoxStream) })
            }),
            options,
        )
    }

    /// A client over any stream source; the seam tests and non-socket transports use.
    pub fn with_connector(connector: Connector, options: ClientOptions) -> Self {
        let (status, _) = watch::channel(Status::Connecting);
        let (events, _) = broadcast::channel(64);
        let (close, _) = watch::channel(false);
        let shared =
            Arc::new(Shared { live: Mutex::new(None), next_id: AtomicU64::new(1), status, events, close, options });
        let task = tokio::spawn(drive(Arc::clone(&shared), connector));
        Self { shared, task }
    }

    pub fn status(&self) -> watch::Receiver<Status> {
        self.shared.status.subscribe()
    }

    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.shared.events.subscribe()
    }

    /// Resolves once connected; `false` on timeout or after `close`.
    pub async fn wait_connected(&self, timeout: Duration) -> bool {
        let mut status = self.status();
        let wait = status.wait_for(|s| matches!(s, Status::Connected | Status::Closed));
        matches!(tokio::time::timeout(timeout, wait).await, Ok(Ok(s)) if *s == Status::Connected)
    }

    pub async fn call<M: Method>(&self, params: M::Params) -> Result<M::Output, CallError> {
        let params = serde_json::to_value(params)?;
        let value = self.call_raw(M::NAME, params).await?;
        Ok(serde_json::from_value(value)?)
    }

    pub fn notify<N: Notification>(&self, params: &N::Params) -> Result<(), CallError> {
        let line = notification_line(N::NAME, params)?;
        let live = locked(&self.shared.live);
        let live = live.as_ref().ok_or(CallError::NotConnected)?;
        live.tx.send(line).map_err(|_| CallError::Disconnected)
    }

    async fn call_raw(&self, method: &str, params: Value) -> Result<Value, CallError> {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply_tx, reply_rx) = oneshot::channel();
        let pending = {
            let live = locked(&self.shared.live);
            let live = live.as_ref().ok_or(CallError::NotConnected)?;
            locked(&live.pending).insert(id, reply_tx);
            let mut msg = Message::blank();
            msg.id = Some(id);
            msg.method = Some(method.into());
            msg.params = Some(params);
            if live.tx.send(msg.line()).is_err() {
                locked(&live.pending).remove(&id);
                return Err(CallError::Disconnected);
            }
            Arc::clone(&live.pending)
        };
        let timeout = self.shared.options.call_timeout;
        match tokio::time::timeout(timeout, reply_rx).await {
            Ok(Ok(outcome)) => Ok(outcome?),
            Ok(Err(_)) => Err(CallError::Disconnected),
            Err(_) => {
                locked(&pending).remove(&id);
                Err(CallError::Timeout(timeout))
            }
        }
    }

    /// Stops reconnecting and drops the connection. Pending calls fail with `Disconnected`.
    pub fn close(&self) {
        let _ = self.shared.close.send(true);
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn drive(shared: Arc<Shared>, connector: Connector) {
    let mut ladder = Ladder::new(shared.options.ladder);
    let mut close = shared.close.subscribe();
    loop {
        let _ = shared.status.send(Status::Connecting);
        let connected = tokio::select! {
            _ = close.wait_for(|c| *c) => break,
            stream = connector() => stream,
        };
        match connected {
            Ok(stream) => {
                ladder.note_connected(Instant::now());
                let (tx, rx) = mpsc::unbounded_channel();
                let pending = Pending::default();
                *locked(&shared.live) = Some(Live { tx, pending: Arc::clone(&pending) });
                let _ = shared.status.send(Status::Connected);
                let events = shared.events.clone();
                let run = pump(stream, rx, |line| dispatch(line, &pending, &events));
                tokio::select! {
                    _ = close.wait_for(|c| *c) => {
                        *locked(&shared.live) = None;
                        locked(&pending).clear();
                        break;
                    }
                    result = run => {
                        if let Err(e) = result {
                            tracing::debug!("rpc client connection ended: {e}");
                        }
                    }
                }
                *locked(&shared.live) = None;
                locked(&pending).clear();
                ladder.note_disconnected(Instant::now());
            }
            Err(e) => tracing::debug!("rpc client connect failed: {e}"),
        }
        let delay = ladder.next_delay();
        let _ = shared.status.send(Status::Waiting { attempt: ladder.attempt(), delay });
        tokio::select! {
            _ = close.wait_for(|c| *c) => break,
            _ = tokio::time::sleep(delay) => {}
        }
    }
    let _ = shared.status.send(Status::Closed);
}

fn dispatch(line: &str, pending: &Pending, events: &broadcast::Sender<Event>) {
    let Ok(msg) = serde_json::from_str::<Message>(line) else { return };
    match (msg.id, msg.method) {
        (Some(id), None) => {
            let outcome = match msg.error {
                Some(e) => Err(e),
                None => Ok(msg.result.unwrap_or(Value::Null)),
            };
            if let Some(reply) = locked(pending).remove(&id) {
                let _ = reply.send(outcome);
            }
        }
        (_, Some(method)) => {
            let _ = events.send(Event { method, params: msg.params.unwrap_or(Value::Null) });
        }
        (None, None) => {}
    }
}
