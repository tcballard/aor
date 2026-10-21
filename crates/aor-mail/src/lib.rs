mod queries;
mod resend;
use aor_db::{Pool, Tx, Uuid};
use aor_jobs::{Claim, Idempotent, Job};
use hmac::{Hmac, Mac};
pub use resend::Resend;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    sync::Arc,
    time::Duration,
};
pub const MIGRATION: &str = include_str!("../migrations/001_mail.sql");
pub const REPLAY_WINDOW: i64 = 23 * 3600;
pub enum Error {
    Configuration,
    Secret,
    Transport,
    Queue(aor_jobs::Error),
    Database(aor_db::Error),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Configuration => "AOR-MAIL-CONFIGURATION",
            Self::Secret => "AOR-MAIL-SECRET",
            Self::Transport => "AOR-MAIL-TRANSPORT",
            Self::Queue(_) => "AOR-MAIL-QUEUE",
            Self::Database(_) => "AOR-MAIL-DATABASE",
        })
    }
}
impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl std::error::Error for Error {}
impl From<aor_jobs::Error> for Error {
    fn from(e: aor_jobs::Error) -> Self {
        Self::Queue(e)
    }
}
impl From<aor_db::Error> for Error {
    fn from(e: aor_db::Error) -> Self {
        Self::Database(e)
    }
}
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Clone)]
pub struct Key(Arc<[u8; 32]>);
impl Key {
    pub fn new(bytes: [u8; 32]) -> Result<Self> {
        if bytes == [0; 32] {
            return Err(Error::Configuration);
        }
        Ok(Self(Arc::new(bytes)))
    }
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = read_secret_file(path, 32)?;
        Self::new(bytes.try_into().map_err(|_| Error::Configuration)?)
    }
    fn token(
        &self,
        id: &str,
        sender: &str,
        recipient: &str,
        purpose: &str,
        expires: i64,
    ) -> String {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(self.0.as_ref()).expect("HMAC accepts 32-byte key");
        mac.update(b"aor-mail-token-v1\0");
        for part in [id, sender, recipient, purpose] {
            mac.update(part.as_bytes());
            mac.update(&[0]);
        }
        mac.update(&expires.to_be_bytes());
        hex(&mac.finalize().into_bytes())
    }
}
pub fn generate_key(path: &Path) -> Result<()> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| Error::Secret)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| Error::Configuration)?;
    file.write_all(&bytes).map_err(|_| Error::Secret)?;
    file.sync_all().map_err(|_| Error::Secret)?;
    Ok(())
}
pub(crate) fn read_secret_file(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let fd = nix::fcntl::open(
        path,
        nix::fcntl::OFlag::O_RDONLY | nix::fcntl::OFlag::O_NOFOLLOW | nix::fcntl::OFlag::O_NONBLOCK,
        nix::sys::stat::Mode::empty(),
    )
    .map_err(|_| Error::Configuration)?;
    let file = std::fs::File::from(fd);
    let meta = file.metadata().map_err(|_| Error::Configuration)?;
    if !meta.is_file() || meta.permissions().mode() & 0o077 != 0 || meta.len() > limit as u64 {
        return Err(Error::Configuration);
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Configuration)?;
    if bytes.is_empty() || bytes.len() > limit {
        return Err(Error::Configuration);
    }
    Ok(bytes)
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn digest(value: &str) -> String {
    hex(&Sha256::digest(value.as_bytes()))
}
fn address(s: &str) -> bool {
    s.len() <= 254
        && !s.is_empty()
        && s.is_ascii()
        && !s
            .bytes()
            .any(|b| b <= 32 || b >= 127 || b"<>\"\\".contains(&b))
        && s.split_once('@')
            .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty() && !b.contains('@'))
}
pub struct IssuedToken(String);
impl IssuedToken {
    pub fn expose(&self) -> &str {
        &self.0
    }
}
#[derive(Clone)]
pub struct Outbox {
    key: Key,
    sender: String,
}
impl Outbox {
    pub fn new(key: Key, sender: String) -> Result<Self> {
        if !address(&sender) {
            return Err(Error::Configuration);
        }
        Ok(Self { key, sender })
    }
    pub async fn enqueue_in(
        &self,
        tx: &mut Tx<'_>,
        recipient: &str,
        purpose: &str,
        expires: i64,
        at: i64,
    ) -> Result<IssuedToken> {
        if !address(recipient)
            || !matches!(purpose, "verify" | "reset")
            || at < 1
            || expires <= at
            || expires - at > 86400
        {
            return Err(Error::Configuration);
        }
        let id = Uuid::new_v4().to_string();
        let token = self
            .key
            .token(&id, &self.sender, recipient, purpose, expires);
        queries::Insert::execute(
            tx,
            id.clone(),
            self.sender.clone(),
            recipient.into(),
            purpose.into(),
            digest(&token),
            expires,
            at,
        )
        .await?;
        aor_jobs::enqueue_in(
            tx,
            &DeliverMailV1 { delivery_id: id },
            "mail",
            at,
            aor_jobs::Options {
                max_attempts: 8,
                run_at: at,
            },
        )
        .await?;
        Ok(IssuedToken(token))
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliverMailV1 {
    pub delivery_id: String,
}
impl Job for DeliverMailV1 {
    const NAME: &'static str = "aor.mail.delivery";
    const VERSION: i64 = 1;
    const IDEMPOTENCY: Idempotent = Idempotent::ByKey;
    fn key(&self) -> String {
        self.delivery_id.clone()
    }
}
pub struct Message {
    sender: String,
    recipient: String,
    subject: &'static str,
    text: String,
    key: String,
}
impl Message {
    pub fn sender(&self) -> &str {
        &self.sender
    }
    pub fn recipient(&self) -> &str {
        &self.recipient
    }
    pub fn subject(&self) -> &str {
        self.subject
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn idempotency_key(&self) -> &str {
        &self.key
    }
}
#[allow(async_fn_in_trait)]
pub trait Transport: Send + Sync {
    async fn send(&self, message: &Message) -> Result<()>;
}
pub struct Worker<T: Transport> {
    pool: Pool,
    key: Key,
    transport: T,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    heartbeat: Duration,
}
impl<T: Transport> Worker<T> {
    pub fn new(pool: Pool, key: Key, transport: T) -> Self {
        Self {
            pool,
            key,
            transport,
            clock: Arc::new(|| aor_db::Utc::now().timestamp()),
            heartbeat: Duration::from_secs(10),
        }
    }
    pub fn with_clock(mut self, clock: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }
    async fn prepare(&self, claim: &Claim) -> Result<Option<Message>> {
        let job = claim.decode::<DeliverMailV1>()?;
        let at = (self.clock)();
        let mut lease = self.pool.acquire().await?;
        let mut tx = lease.begin().await?;
        aor_jobs::fence_in(&mut tx, claim, at).await?;
        let row = queries::Load::query(&mut tx, job.delivery_id.clone())
            .await?
            .pop()
            .ok_or(Error::Configuration)?;
        if row.state != "pending" {
            if matches!(row.state.as_str(), "delivered" | "expired") {
                aor_jobs::complete_in(&mut tx, claim, at).await?;
            } else {
                aor_jobs::fail_permanently_in(&mut tx, claim, at).await?;
            }
            tx.commit().await?;
            return Ok(None);
        }
        let disposition = if at < row.created_at {
            Some("clock_invalid")
        } else if at >= row.expires_at {
            Some("expired")
        } else if row.first_attempt_at > 0
            && (at < row.first_attempt_at || at - row.first_attempt_at >= REPLAY_WINDOW)
        {
            Some("ambiguous")
        } else {
            None
        };
        if let Some(state) = disposition {
            queries::State::execute(&mut tx, state.into(), row.id).await?;
            if state == "expired" {
                aor_jobs::complete_in(&mut tx, claim, at).await?;
            } else {
                aor_jobs::fail_permanently_in(&mut tx, claim, at).await?;
            }
            tx.commit().await?;
            return Ok(None);
        }
        let token = self.key.token(
            &row.id,
            &row.sender,
            &row.recipient,
            &row.purpose,
            row.expires_at,
        );
        if digest(&token) != row.token_digest {
            return Err(Error::Secret);
        }
        if row.first_attempt_at == 0 {
            queries::Start::execute(&mut tx, at, row.id.clone()).await?;
        }
        tx.commit().await?;
        let subject = match row.purpose.as_str() {
            "verify" => "Verify your Registry account",
            "reset" => "Reset your Registry password",
            _ => return Err(Error::Configuration),
        };
        Ok(Some(Message {
            sender: row.sender,
            recipient: row.recipient,
            subject,
            text: format!(
                "Your Registry {} code is:\n\n{}\n\nIf you did not request this, ignore this email.\n",
                if row.purpose == "verify" {
                    "verification"
                } else {
                    "password reset"
                },
                token
            ),
            key: format!("aor-mail/{}", row.id),
        }))
    }
    pub async fn process(&self, claim: &Claim) -> Result<()> {
        let Some(message) = self.prepare(claim).await? else {
            return Ok(());
        };
        let send = self.transport.send(&message);
        tokio::pin!(send);
        let mut heartbeat = tokio::time::interval(self.heartbeat);
        heartbeat.tick().await;
        let result = loop {
            tokio::select! {result=&mut send=>break result,_=heartbeat.tick()=>{aor_jobs::renew(&self.pool,claim,(self.clock)(),60).await?;}}
        };
        if result.is_err() {
            aor_jobs::fail(
                &self.pool,
                claim,
                (self.clock)(),
                aor_jobs::Failure::Dependency,
            )
            .await?;
            return Err(Error::Transport);
        }
        let at = (self.clock)();
        let job = claim.decode::<DeliverMailV1>()?;
        let mut lease = self.pool.acquire().await?;
        let mut tx = lease.begin().await?;
        aor_jobs::fence_in(&mut tx, claim, at).await?;
        queries::State::execute(&mut tx, "delivered".into(), job.delivery_id).await?;
        aor_jobs::complete_in(&mut tx, claim, at).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn run_claim(&self, claim: &Claim) -> Result<()> {
        let at = (self.clock)();
        if claim.job_type() != DeliverMailV1::NAME
            || claim.payload_version() != DeliverMailV1::VERSION
        {
            aor_jobs::incompatible(&self.pool, claim, at).await?;
            return Ok(());
        }
        match self.process(claim).await {
            Err(Error::Secret | Error::Configuration) => {
                let mut lease = self.pool.acquire().await?;
                let mut tx = lease.begin().await?;
                aor_jobs::fail_permanently_in(&mut tx, claim, (self.clock)()).await?;
                tx.commit().await?;
                Err(Error::Secret)
            }
            result => result,
        }
    }
    pub async fn work_once(&self) -> Result<bool> {
        let Some(claim) = aor_jobs::claim(&self.pool, "mail", (self.clock)(), 60).await? else {
            return Ok(false);
        };
        self.run_claim(&claim).await?;
        Ok(true)
    }
}
#[cfg(test)]
#[path = "../tests/support/lifecycle.rs"]
mod lifecycle;
