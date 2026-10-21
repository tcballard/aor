mod queries;
use aor_db::{Dialect, Pool, Tx, Uuid};
use serde::{Serialize, de::DeserializeOwned};
use std::fmt;

pub const MIGRATION: &str = include_str!("../migrations/001_jobs.sql");
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
#[derive(Clone, Copy)]
pub enum Idempotent {
    ByKey,
    Natural,
    AtLeastOnceTolerated(&'static str),
}
pub trait Job: Serialize + DeserializeOwned {
    const NAME: &'static str;
    const VERSION: i64;
    const IDEMPOTENCY: Idempotent;
    fn key(&self) -> String;
}
#[derive(Debug)]
pub enum Error {
    Database(aor_db::Error),
    Invalid,
    Stale,
    Incompatible,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Database(_) => "AOR-JOB-DATABASE",
            Self::Invalid => "AOR-JOB-INVALID",
            Self::Stale => "AOR-JOB-STALE",
            Self::Incompatible => "AOR-JOB-INCOMPATIBLE",
        })
    }
}
impl std::error::Error for Error {}
impl From<aor_db::Error> for Error {
    fn from(value: aor_db::Error) -> Self {
        Self::Database(value)
    }
}
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Copy)]
pub struct Options {
    pub max_attempts: i64,
    pub run_at: i64,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            run_at: 0,
        }
    }
}
pub struct Claim {
    id: String,
    job_type: String,
    payload_version: i64,
    payload: String,
    attempt: i64,
    max_attempts: i64,
    generation: i64,
}
impl Claim {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn job_type(&self) -> &str {
        &self.job_type
    }
    pub fn payload_version(&self) -> i64 {
        self.payload_version
    }
    pub fn attempt(&self) -> i64 {
        self.attempt
    }
    pub fn decode<J: Job>(&self) -> Result<J> {
        if self.job_type != J::NAME || self.payload_version != J::VERSION {
            return Err(Error::Incompatible);
        }
        serde_json::from_str(&self.payload).map_err(|_| Error::Invalid)
    }
}
fn label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
fn clock(at: i64) -> Result<()> {
    if !(0..=i64::MAX - 86400).contains(&at) {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn expiry(at: i64, seconds: i64) -> Result<i64> {
    clock(at)?;
    if !(1..=3600).contains(&seconds) {
        return Err(Error::Invalid);
    }
    Ok(at + seconds)
}
fn affected(rows: u64) -> Result<()> {
    if rows == 1 { Ok(()) } else { Err(Error::Stale) }
}
pub async fn enqueue_in<J: Job>(
    tx: &mut Tx<'_>,
    job: &J,
    queue: &str,
    at: i64,
    options: Options,
) -> Result<String> {
    clock(at)?;
    let key = job.key();
    if !label(J::NAME)
        || J::VERSION < 1
        || !label(queue)
        || key.is_empty()
        || key.len() > 256
        || key.chars().any(char::is_control)
        || !(1..=100).contains(&options.max_attempts)
        || options.run_at < 0
        || matches!(J::IDEMPOTENCY, Idempotent::AtLeastOnceTolerated(reason) if reason.trim().is_empty())
    {
        return Err(Error::Invalid);
    }
    let payload = serde_json::to_string(job).map_err(|_| Error::Invalid)?;
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(Error::Invalid);
    }
    queries::Insert::execute(
        tx,
        Uuid::new_v4().to_string(),
        J::NAME.into(),
        J::VERSION,
        payload,
        key.clone(),
        queue.into(),
        options.max_attempts,
        options.run_at.max(at),
        at,
    )
    .await?;
    Ok(queries::ByKey::query(tx, J::NAME.into(), key)
        .await?
        .pop()
        .ok_or(Error::Invalid)?
        .id)
}
pub async fn claim(pool: &Pool, queue: &str, at: i64, lease_seconds: i64) -> Result<Option<Claim>> {
    let expires = expiry(at, lease_seconds)?;
    if !label(queue) {
        return Err(Error::Invalid);
    }
    let mut lease = pool.acquire().await?;
    let mut tx = lease.begin().await?;
    queries::ExhaustedDue::execute(&mut tx, at, queue.into()).await?;
    if pool.dialect() == Dialect::Postgres {
        let row = queries::ClaimPostgres::query(&mut tx, expires, at, queue.into())
            .await?
            .pop();
        tx.commit().await?;
        return Ok(row.map(|row| Claim {
            id: row.id,
            job_type: row.job_type,
            payload_version: row.payload_version,
            payload: row.payload,
            attempt: row.attempt,
            max_attempts: row.max_attempts,
            generation: row.lease_generation,
        }));
    }
    let Some(row) = queries::DueLocal::query(&mut tx, queue.into(), at)
        .await?
        .pop()
    else {
        tx.commit().await?;
        return Ok(None);
    };
    affected(
        queries::ClaimRow::execute(&mut tx, expires, at, row.id.clone(), row.lease_generation)
            .await?,
    )?;
    tx.commit().await?;
    Ok(Some(Claim {
        id: row.id,
        job_type: row.job_type,
        payload_version: row.payload_version,
        payload: row.payload,
        attempt: row.attempt + 1,
        max_attempts: row.max_attempts,
        generation: row.lease_generation + 1,
    }))
}
pub async fn fence_in(tx: &mut Tx<'_>, claim: &Claim, at: i64) -> Result<()> {
    clock(at)?;
    affected(queries::Fence::execute(tx, at, claim.id.clone(), claim.generation).await?)
}
pub async fn complete_in(tx: &mut Tx<'_>, claim: &Claim, at: i64) -> Result<()> {
    clock(at)?;
    affected(queries::Complete::execute(tx, at, claim.id.clone(), claim.generation).await?)
}
pub async fn renew(pool: &Pool, claim: &Claim, at: i64, seconds: i64) -> Result<()> {
    let expires = expiry(at, seconds)?;
    let mut lease = pool.acquire().await?;
    let mut tx = lease.begin().await?;
    affected(
        queries::Renew::execute(&mut tx, expires, at, claim.id.clone(), claim.generation).await?,
    )?;
    tx.commit().await?;
    Ok(())
}
#[derive(Clone, Copy)]
pub enum Failure {
    Handler,
    Payload,
    Dependency,
}
impl Failure {
    fn code(self) -> &'static str {
        match self {
            Self::Handler => "AOR-JOB-HANDLER",
            Self::Payload => "AOR-JOB-PAYLOAD",
            Self::Dependency => "AOR-JOB-DEPENDENCY",
        }
    }
}
pub async fn fail(pool: &Pool, claim: &Claim, at: i64, failure: Failure) -> Result<()> {
    clock(at)?;
    let base = 1_i64 << claim.attempt.min(11);
    let jitter = claim
        .id
        .bytes()
        .fold(0_u64, |a, b| a.wrapping_mul(31).wrapping_add(b.into()))
        % (base as u64 + 1);
    let delay = (base + jitter as i64).min(3600);
    let state = if claim.attempt >= claim.max_attempts {
        "failed"
    } else {
        "queued"
    };
    let mut lease = pool.acquire().await?;
    let mut tx = lease.begin().await?;
    affected(
        queries::Retry::execute(
            &mut tx,
            state.into(),
            at + delay,
            failure.code().into(),
            at,
            claim.id.clone(),
            claim.generation,
        )
        .await?,
    )?;
    tx.commit().await?;
    Ok(())
}
pub async fn incompatible(pool: &Pool, claim: &Claim, at: i64) -> Result<()> {
    clock(at)?;
    let mut lease = pool.acquire().await?;
    let mut tx = lease.begin().await?;
    affected(
        queries::Incompatible::execute(&mut tx, at, claim.id.clone(), claim.generation).await?,
    )?;
    tx.commit().await?;
    Ok(())
}
pub use queries::{InspectRow as Inspection, ListRow as Summary};
pub async fn inspect(pool: &Pool, id: &str) -> Result<Option<Inspection>> {
    let mut lease = pool.acquire().await?;
    Ok(queries::Inspect::query(&mut lease, id.into()).await?.pop())
}
pub async fn list(pool: &Pool, queue: &str, limit: i64) -> Result<Vec<Summary>> {
    if !label(queue) || !(1..=1000).contains(&limit) {
        return Err(Error::Invalid);
    }
    let mut lease = pool.acquire().await?;
    Ok(queries::List::query(&mut lease, queue.into(), limit).await?)
}
pub async fn requeue(pool: &Pool, id: &str, at: i64) -> Result<()> {
    clock(at)?;
    let mut lease = pool.acquire().await?;
    affected(queries::Requeue::execute(&mut lease, at, id.into()).await?)
}
pub async fn park(pool: &Pool, id: &str, at: i64) -> Result<()> {
    clock(at)?;
    let mut lease = pool.acquire().await?;
    affected(queries::Park::execute(&mut lease, at, id.into()).await?)
}
pub async fn migrate(pool: &Pool) -> std::result::Result<usize, Box<dyn std::error::Error>> {
    let migrations = aor_migrate::embedded(&[("001_jobs.sql", MIGRATION)], pool.dialect())?;
    Ok(aor_migrate::apply(pool, &migrations).await?)
}
