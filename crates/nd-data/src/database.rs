use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use futures_channel::oneshot;
use rusqlite::{Connection, params_from_iter};
use serde::de::DeserializeOwned;

use crate::migrate::{self, Migration};
use crate::value::{Row, RunResult, Statement, StepResult, Value};
use crate::{Error, Result};

#[derive(Clone, Debug)]
pub struct OpenOptions {
    /// `None` opens an in-memory database.
    pub path: Option<PathBuf>,
    pub create_parent_dirs: bool,
    /// Write-ahead logging, so readers in other processes do not block the writer. Ignored
    /// for in-memory databases.
    pub wal: bool,
    pub busy_timeout: Duration,
    pub foreign_keys: bool,
}

impl OpenOptions {
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self { path: Some(path.into()), ..Self::memory() }
    }

    pub fn memory() -> Self {
        Self {
            path: None,
            create_parent_dirs: true,
            wal: true,
            busy_timeout: Duration::from_secs(5),
            foreign_keys: true,
        }
    }
}

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

enum Message {
    Run(Job),
    Close(oneshot::Sender<()>),
}

/// A handle to the database thread. Clones share the one connection; the connection closes
/// when the last handle drops or [`Database::close`] runs.
#[derive(Clone)]
pub struct Database {
    jobs: Arc<mpsc::Sender<Message>>,
}

impl Database {
    pub async fn open(options: OpenOptions) -> Result<Self> {
        let (jobs, rx) = mpsc::channel::<Message>();
        let (ready_tx, ready_rx) = oneshot::channel();
        std::thread::Builder::new().name("nd-data".into()).spawn(move || {
            let mut conn = match connect(&options) {
                Ok(conn) => {
                    let _ = ready_tx.send(Ok(()));
                    conn
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            while let Ok(message) = rx.recv() {
                match message {
                    Message::Run(job) => job(&mut conn),
                    Message::Close(done) => {
                        drop(conn);
                        let _ = done.send(());
                        return;
                    }
                }
            }
        })?;
        ready_rx.await.map_err(|_| Error::Closed)??;
        Ok(Self { jobs: Arc::new(jobs) })
    }

    pub async fn open_memory() -> Result<Self> {
        Self::open(OpenOptions::memory()).await
    }

    /// Runs `f` on the database thread with the raw connection. The escape hatch for anything
    /// the typed calls do not cover; a panic inside `f` comes back as [`Error::Panicked`].
    pub fn with<T, F>(&self, f: F) -> impl Future<Output = Result<T>> + Send + use<T, F>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        let job: Job = Box::new(move |conn| {
            let out =
                catch_unwind(AssertUnwindSafe(|| f(conn))).unwrap_or_else(|p| Err(Error::Panicked(panic_text(&p))));
            let _ = tx.send(out);
        });
        let sent = self.jobs.send(Message::Run(job)).is_ok();
        async move {
            if !sent {
                return Err(Error::Closed);
            }
            rx.await.map_err(|_| Error::Closed)?
        }
    }

    pub async fn query_as<T: DeserializeOwned>(&self, sql: &str, params: Vec<Value>) -> Result<Vec<T>> {
        self.query(sql, params).await?.iter().map(Row::decode).collect()
    }

    /// Applies pending migrations and returns the schema version the database is now at.
    pub async fn migrate(&self, migrations: &'static [Migration]) -> Result<u32> {
        self.with(move |conn| migrate::run(conn, migrations)).await
    }

    /// Closes the connection after the jobs already queued have run. Every clone of this
    /// handle fails with [`Error::Closed`] afterwards.
    pub async fn close(self) -> Result<()> {
        let (done, closed) = oneshot::channel();
        if self.jobs.send(Message::Close(done)).is_ok() {
            let _ = closed.await;
        }
        Ok(())
    }
}

/// What an ORM adapter targets: the three calls `@nativedesktop/data`'s `SqliteExecutor` has.
pub trait SqliteExecutor: Send + Sync {
    fn query(&self, sql: &str, params: Vec<Value>) -> impl Future<Output = Result<Vec<Row>>> + Send;
    fn mutate(&self, sql: &str, params: Vec<Value>) -> impl Future<Output = Result<RunResult>> + Send;
    /// Runs every step in one transaction; any failure rolls all of them back.
    fn transaction(&self, steps: Vec<Statement>) -> impl Future<Output = Result<Vec<StepResult>>> + Send;
}

impl SqliteExecutor for Database {
    fn query(&self, sql: &str, params: Vec<Value>) -> impl Future<Output = Result<Vec<Row>>> + Send {
        let sql = sql.to_owned();
        self.with(move |conn| run_query(conn, &sql, &params))
    }

    fn mutate(&self, sql: &str, params: Vec<Value>) -> impl Future<Output = Result<RunResult>> + Send {
        let sql = sql.to_owned();
        self.with(move |conn| run_mutate(conn, &sql, &params))
    }

    fn transaction(&self, steps: Vec<Statement>) -> impl Future<Output = Result<Vec<StepResult>>> + Send {
        self.with(move |conn| {
            let tx = conn.transaction()?;
            let mut out = Vec::with_capacity(steps.len());
            for step in &steps {
                out.push(run_step(&tx, step)?);
            }
            tx.commit()?;
            Ok(out)
        })
    }
}

fn connect(options: &OpenOptions) -> Result<Connection> {
    let conn = match &options.path {
        Some(path) => {
            if options.create_parent_dirs
                && let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty())
            {
                std::fs::create_dir_all(dir)?;
            }
            Connection::open(path)?
        }
        None => Connection::open_in_memory()?,
    };
    conn.busy_timeout(options.busy_timeout)?;
    conn.pragma_update(None, "foreign_keys", options.foreign_keys)?;
    if options.wal && options.path.is_some() {
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get::<_, String>(0))?;
    }
    Ok(conn)
}

fn run_query(conn: &Connection, sql: &str, params: &[Value]) -> Result<Vec<Row>> {
    let mut stmt = conn.prepare_cached(sql)?;
    collect_rows(&mut stmt, params)
}

fn collect_rows(stmt: &mut rusqlite::Statement<'_>, params: &[Value]) -> Result<Vec<Row>> {
    let columns: Arc<[String]> = stmt.column_names().into_iter().map(str::to_owned).collect();
    let width = columns.len();
    let mut rows = stmt.query(params_from_iter(params))?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let values = (0..width).map(|i| row.get_ref(i).map(Value::from_ref)).collect::<rusqlite::Result<_>>()?;
        out.push(Row { columns: Arc::clone(&columns), values });
    }
    Ok(out)
}

fn run_mutate(conn: &Connection, sql: &str, params: &[Value]) -> Result<RunResult> {
    let mut stmt = conn.prepare_cached(sql)?;
    let changes = stmt.execute(params_from_iter(params))? as u64;
    Ok(RunResult { changes, last_insert_rowid: conn.last_insert_rowid() })
}

fn run_step(conn: &Connection, step: &Statement) -> Result<StepResult> {
    let mut stmt = conn.prepare_cached(&step.sql)?;
    if stmt.column_count() > 0 {
        Ok(StepResult::Rows(collect_rows(&mut stmt, &step.params)?))
    } else {
        let changes = stmt.execute(params_from_iter(&step.params))? as u64;
        Ok(StepResult::Run(RunResult { changes, last_insert_rowid: conn.last_insert_rowid() }))
    }
}

fn panic_text(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic".into())
}
