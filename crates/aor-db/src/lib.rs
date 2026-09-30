//! Thin bounded pools over wire/storage foundations. All transaction work uses one lease.
mod value;
pub use aor_db_macros::sql;
pub use aor_sql::Dialect;
pub use chrono::{DateTime, Utc};
use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
pub use uuid::Uuid;
pub use value::{Decode, IntoValue, Row, Value, parameter};
#[derive(Debug)]
pub enum Error {
    Postgres(tokio_postgres::Error),
    Sqlite(rusqlite::Error),
    Timeout,
    Closed,
    Dialect,
    Decode(String),
    Configuration(String),
    Task(String),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postgres(e) => write!(f, "PostgreSQL: {e}"),
            Self::Sqlite(e) => write!(f, "SQLite: {e}"),
            Self::Timeout => write!(f, "database deadline exceeded"),
            Self::Closed => write!(f, "database pool closed"),
            Self::Dialect => write!(f, "query dialect does not match connection"),
            Self::Decode(e) | Self::Configuration(e) | Self::Task(e) => f.write_str(e),
        }
    }
}
impl std::error::Error for Error {}
impl From<tokio_postgres::Error> for Error {
    fn from(e: tokio_postgres::Error) -> Self {
        Self::Postgres(e)
    }
}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug)]
pub struct QueryResult {
    pub rows: Vec<Row>,
    pub affected: u64,
}
#[allow(async_fn_in_trait)]
pub trait Executor {
    async fn query(&mut self, dialect: Dialect, sql: &str, params: &[Value])
    -> Result<QueryResult>;
}
#[derive(Clone, Debug)]
pub struct PoolOptions {
    pub max_connections: usize,
    pub acquire_timeout: Duration,
    pub query_timeout: Duration,
    pub statement_cache_capacity: usize,
}
impl Default for PoolOptions {
    fn default() -> Self {
        Self {
            max_connections: 8,
            acquire_timeout: Duration::from_secs(5),
            query_timeout: Duration::from_secs(15),
            statement_cache_capacity: 128,
        }
    }
}
enum Backend {
    Postgres(Box<tokio_postgres::Config>),
    Sqlite(std::path::PathBuf),
}
struct Inner {
    backend: Backend,
    options: PoolOptions,
    slots: Arc<Semaphore>,
    idle: Mutex<Vec<Connection>>,
}
#[derive(Clone)]
pub struct Pool(Arc<Inner>);
enum Connection {
    Postgres {
        client: tokio_postgres::Client,
        task: tokio::task::JoinHandle<()>,
        cache: BTreeMap<String, tokio_postgres::Statement>,
    },
    Sqlite(Arc<Mutex<rusqlite::Connection>>),
}
impl Drop for Connection {
    fn drop(&mut self) {
        if let Self::Postgres { task, .. } = self {
            task.abort()
        }
    }
}
pub struct Lease {
    pool: Arc<Inner>,
    connection: Option<Connection>,
    _permit: OwnedSemaphorePermit,
    poisoned: bool,
    in_transaction: bool,
    session_dirty: bool,
}
impl Pool {
    fn new(backend: Backend, options: PoolOptions) -> Result<Self> {
        if options.max_connections == 0
            || options.max_connections > 256
            || options.acquire_timeout.is_zero()
            || options.query_timeout.is_zero()
            || options.statement_cache_capacity == 0
        {
            return Err(Error::Configuration("invalid pool limits".into()));
        }
        Ok(Self(Arc::new(Inner {
            slots: Arc::new(Semaphore::new(options.max_connections)),
            backend,
            options,
            idle: Mutex::new(Vec::new()),
        })))
    }
    /// This transport is for Unix sockets or loopback PostgreSQL. TLS termination is external.
    pub fn postgres(url: &str, options: PoolOptions) -> Result<Self> {
        let config = url.parse::<tokio_postgres::Config>()?;
        for host in config.get_hosts() {
            if let tokio_postgres::config::Host::Tcp(host) = host {
                if host != "localhost"
                    && host
                        .parse::<std::net::IpAddr>()
                        .map_or(true, |ip| !ip.is_loopback())
                {
                    return Err(Error::Configuration(
                        "unencrypted PostgreSQL must use loopback or a Unix socket".into(),
                    ));
                }
            }
        }
        Self::new(Backend::Postgres(Box::new(config)), options)
    }
    pub fn sqlite(path: impl AsRef<std::path::Path>, mut options: PoolOptions) -> Result<Self> {
        options.max_connections = 1;
        Self::new(Backend::Sqlite(path.as_ref().to_owned()), options)
    }
    pub fn dialect(&self) -> Dialect {
        match self.0.backend {
            Backend::Postgres(_) => Dialect::Postgres,
            Backend::Sqlite(_) => Dialect::Sqlite,
        }
    }
    pub fn close(&self) {
        self.0.slots.close();
        self.0.idle.lock().unwrap().clear();
    }
    pub async fn acquire(&self) -> Result<Lease> {
        tokio::time::timeout(self.0.options.acquire_timeout, async {
            let permit = self
                .0
                .slots
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| Error::Closed)?;
            let idle = self.0.idle.lock().unwrap().pop();
            let connection = match idle {
                Some(c) => c,
                None => match &self.0.backend {
                    Backend::Postgres(config) => {
                        let (client, connection) = config.connect(tokio_postgres::NoTls).await?;
                        let task = tokio::spawn(async move {
                            let _ = connection.await;
                        });
                        Connection::Postgres {
                            client,
                            task,
                            cache: BTreeMap::new(),
                        }
                    }
                    Backend::Sqlite(path) => {
                        let path = path.clone();
                        let busy = self.0.options.query_timeout;
                        let conn = tokio::task::spawn_blocking(move || -> Result<_> {
                            let conn = rusqlite::Connection::open(path)?;
                            conn.busy_timeout(busy)?;
                            conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;")?;
                            Ok(conn)
                        })
                        .await
                        .map_err(|e| Error::Task(e.to_string()))??;
                        Connection::Sqlite(Arc::new(Mutex::new(conn)))
                    }
                },
            };
            Ok(Lease {
                pool: self.0.clone(),
                connection: Some(connection),
                _permit: permit,
                poisoned: false,
                in_transaction: false,
                session_dirty: false,
            })
        })
        .await
        .map_err(|_| Error::Timeout)?
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if !self.session_dirty
            && !self.poisoned
            && !self.in_transaction
            && !self.pool.slots.is_closed()
        {
            if let Some(c) = self.connection.take() {
                let closed = matches!(&c,Connection::Postgres{client,..}if client.is_closed());
                if !closed {
                    self.pool.idle.lock().unwrap().push(c)
                }
            }
        }
    }
}
impl Lease {
    pub fn dialect(&self) -> Dialect {
        match self.pool.backend {
            Backend::Postgres(_) => Dialect::Postgres,
            Backend::Sqlite(_) => Dialect::Sqlite,
        }
    }
    /// Discard rather than reuse a connection after uncertain session state (e.g. lock failure).
    pub fn session_dirty(&mut self, dirty: bool) {
        self.session_dirty = dirty;
    }
    pub fn discard(&mut self) {
        self.poisoned = true;
    }
    pub async fn begin(&mut self) -> Result<Tx<'_>> {
        if self.in_transaction || self.poisoned {
            return Err(Error::Configuration(
                "connection already in transaction or discarded".into(),
            ));
        }
        self.in_transaction = true;
        self.batch("BEGIN").await?;
        Ok(Tx {
            lease: self,
            finished: false,
        })
    }
    /// Migration-only SQL batches. Application escape hatches should use sql_unchecked!.
    pub async fn batch(&mut self, sql: &str) -> Result<()> {
        let sql = sql.to_owned();
        let duration = self.pool.options.query_timeout;
        self.poisoned = true;
        let result = tokio::time::timeout(duration, async {
            match self.connection.as_mut().ok_or(Error::Closed)? {
                Connection::Postgres { client, cache, .. } => {
                    cache.clear();
                    client.batch_execute(&sql).await?;
                    Ok(())
                }
                Connection::Sqlite(conn) => {
                    let conn = conn.clone();
                    tokio::task::spawn_blocking(move || conn.lock().unwrap().execute_batch(&sql))
                        .await
                        .map_err(|e| Error::Task(e.to_string()))??;
                    Ok(())
                }
            }
        })
        .await
        .map_err(|_| Error::Timeout)?;
        if result.is_ok() {
            self.poisoned = false
        }
        result
    }
}
impl Executor for Lease {
    async fn query(
        &mut self,
        dialect: Dialect,
        sql: &str,
        params: &[Value],
    ) -> Result<QueryResult> {
        if self.poisoned {
            return Err(Error::Closed);
        }
        if self.dialect() != dialect {
            return Err(Error::Dialect);
        }
        record_query(sql);
        self.poisoned = true;
        let capacity = self.pool.options.statement_cache_capacity;
        let duration = self.pool.options.query_timeout;
        let result = tokio::time::timeout(duration, async {
            match self.connection.as_mut().ok_or(Error::Closed)? {
                Connection::Postgres { client, cache, .. } => {
                    if !cache.contains_key(sql) {
                        let statement = client.prepare(sql).await?;
                        if cache.len() >= capacity {
                            cache.clear()
                        }
                        cache.insert(sql.into(), statement);
                    }
                    let statement = &cache[sql];
                    let refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                        params.iter().map(|v| v as _).collect();
                    if statement.columns().is_empty() {
                        let affected = client.execute(statement, &refs).await?;
                        Ok(QueryResult {
                            rows: Vec::new(),
                            affected,
                        })
                    } else {
                        let rows = client
                            .query(statement, &refs)
                            .await?
                            .into_iter()
                            .map(value::pg_row)
                            .collect::<Result<Vec<_>>>()?;
                        Ok(QueryResult {
                            affected: rows.len() as u64,
                            rows,
                        })
                    }
                }
                Connection::Sqlite(conn) => {
                    let conn = conn.clone();
                    let sql = sql.to_owned();
                    let params: Vec<_> = params.iter().map(value::sqlite_value).collect();
                    tokio::task::spawn_blocking(move || -> Result<QueryResult> {
                        let conn = conn.lock().unwrap();
                        let mut statement = conn.prepare_cached(&sql)?;
                        let n = statement.column_count();
                        if n == 0 {
                            let affected = statement.execute(rusqlite::params_from_iter(params))?;
                            return Ok(QueryResult {
                                rows: Vec::new(),
                                affected: affected as u64,
                            });
                        }
                        let mut cursor = statement.query(rusqlite::params_from_iter(params))?;
                        let mut rows = Vec::new();
                        while let Some(row) = cursor.next()? {
                            let mut values = Vec::new();
                            for i in 0..n {
                                use rusqlite::types::ValueRef as S;
                                values.push(match row.get_ref(i)? {
                                    S::Null => Value::Null,
                                    S::Integer(n) => Value::I64(n),
                                    S::Text(s) => Value::Text(
                                        std::str::from_utf8(s)
                                            .map_err(|e| Error::Decode(e.to_string()))?
                                            .into(),
                                    ),
                                    S::Blob(b) => Value::Bytes(b.to_vec()),
                                    S::Real(_) => {
                                        return Err(Error::Decode(
                                            "floating point is outside the checked type subset"
                                                .into(),
                                        ));
                                    }
                                })
                            }
                            rows.push(Row(values));
                        }
                        Ok(QueryResult {
                            affected: rows.len() as u64,
                            rows,
                        })
                    })
                    .await
                    .map_err(|e| Error::Task(e.to_string()))?
                }
            }
        })
        .await
        .map_err(|_| Error::Timeout)?;
        // SQL errors are safe to reuse outside a transaction; transaction errors require rollback.
        self.poisoned = false;
        result
    }
}
/// Dropping an unfinished transaction discards its connection, forcing server rollback.
/// Commit/rollback consume the handle; no repository can commit through Executor.
pub struct Tx<'a> {
    lease: &'a mut Lease,
    finished: bool,
}
impl Tx<'_> {
    pub async fn commit(mut self) -> Result<()> {
        self.lease.batch("COMMIT").await?;
        self.lease.in_transaction = false;
        self.finished = true;
        Ok(())
    }
    pub async fn rollback(mut self) -> Result<()> {
        self.lease.batch("ROLLBACK").await?;
        self.lease.in_transaction = false;
        self.finished = true;
        Ok(())
    }
}
impl Drop for Tx<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.lease.poisoned = true;
        }
    }
}
impl Executor for Tx<'_> {
    async fn query(
        &mut self,
        dialect: Dialect,
        sql: &str,
        params: &[Value],
    ) -> Result<QueryResult> {
        self.lease.query(dialect, sql, params).await
    }
}
#[macro_export]
macro_rules! sql_unchecked {
    ($connection:expr,$dialect:expr,$sql:expr,$params:expr) => {{ $crate::Executor::query($connection, $dialect, $sql, $params) }};
}
#[derive(Clone, Debug, Default)]
pub struct QueryCounter(Arc<Mutex<BTreeMap<String, usize>>>);
tokio::task_local! {static COUNTER:QueryCounter;}
impl QueryCounter {
    pub async fn scope<F: std::future::Future>(&self, future: F) -> F::Output {
        COUNTER.scope(self.clone(), future).await
    }
    pub fn repeated(&self, threshold: usize) -> Vec<(String, usize)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, count)| **count >= threshold)
            .map(|(sql, count)| (sql.clone(), *count))
            .collect()
    }
    pub fn total(&self) -> usize {
        self.0.lock().unwrap().values().sum()
    }
}
fn record_query(sql: &str) {
    let _ = COUNTER.try_with(|c| {
        *c.0.lock()
            .unwrap()
            .entry(sql.split_whitespace().collect::<Vec<_>>().join(" "))
            .or_default() += 1;
    });
}
