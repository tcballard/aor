//! Server-side authentication. Secrets are never serialized or included in Debug output.
mod queries;
use aor_db::{Pool, Tx, Uuid};
use argon2::{
    Algorithm, Argon2, Params, PasswordHasher, PasswordVerifier, Version,
    password_hash::{PasswordHash, SaltString},
};
use queries::*;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug)]
pub enum Error {
    Configuration,
    InvalidInput,
    Unauthenticated,
    Csrf,
    Forbidden,
    Database(aor_db::Error),
    Crypto,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Configuration => "AUTH_CONFIGURATION",
            Self::InvalidInput => "AUTH_INVALID_INPUT",
            Self::Unauthenticated => "AUTH_INVALID_CREDENTIALS",
            Self::Csrf => "CSRF_REJECTED",
            Self::Forbidden => "AUTH_FORBIDDEN",
            Self::Database(_) => "AUTH_DATABASE",
            Self::Crypto => "AUTH_CRYPTO",
        })
    }
}
impl std::error::Error for Error {}
impl From<aor_db::Error> for Error {
    fn from(e: aor_db::Error) -> Self {
        Self::Database(e)
    }
}
#[derive(Clone, Debug)]
pub struct Config {
    pub public_origin: String,
    pub production: bool,
    pub secure_cookie: bool,
    pub strict_cookie: bool,
    pub idle_seconds: i64,
    pub absolute_seconds: i64,
    pub registration_enabled: bool,
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        let origin = &self.public_origin;
        let prefix = if self.production || origin.starts_with("https://") {
            "https://"
        } else {
            "http://"
        };
        let host = origin.strip_prefix(prefix).ok_or(Error::Configuration)?;
        let (hostname, port) = host
            .split_once(':')
            .map_or((host, None), |(h, p)| (h, Some(p)));
        if hostname.is_empty()
            || hostname.len() > 253
            || hostname.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
            || port.is_some_and(|p| p.parse::<u16>().map_or(true, |n| n == 0))
            || self.secure_cookie && prefix != "https://"
        {
            return Err(Error::Configuration);
        }
        if host.is_empty()
            || host.contains(['/', '?', '#', '@', '\\'])
            || host.bytes().any(|b| b <= 32 || b >= 127)
            || self.production && !self.secure_cookie
            || self.idle_seconds < 1
            || self.absolute_seconds < self.idle_seconds
            || self.absolute_seconds > 366 * 86400
        {
            return Err(Error::Configuration);
        }
        if !(self.production
            || self.secure_cookie
            || hostname == "localhost"
            || hostname == "127.0.0.1")
        {
            return Err(Error::Configuration);
        }
        Ok(())
    }
    pub fn cookie_name(&self) -> &'static str {
        if self.secure_cookie {
            "__Host-aor_session"
        } else {
            "aor_dev_session"
        }
    }
    pub fn csrf_cookie_name(&self) -> &'static str {
        if self.secure_cookie {
            "__Host-aor_csrf"
        } else {
            "aor_dev_csrf"
        }
    }
}
#[derive(Clone)]
pub struct Principal {
    user_id: Uuid,
    epoch: i64,
    credential_hash: String,
    scopes: Option<Vec<String>>,
}
impl Principal {
    pub fn user_id(&self) -> Uuid {
        self.user_id
    }
    pub fn permits(&self, resource: &str, action: &str) -> bool {
        self.scopes
            .as_ref()
            .is_none_or(|scopes| scopes.iter().any(|s| s == &format!("{resource}:{action}")))
    }
    pub fn is_session(&self) -> bool {
        self.scopes.is_none()
    }
}
#[derive(Clone)]
pub struct Session {
    token_hash: String,
    csrf_hash: String,
}
#[derive(Clone)]
pub struct Identity {
    principal: Principal,
    session: Option<Session>,
}
impl Identity {
    pub fn principal(&self) -> &Principal {
        &self.principal
    }
    pub fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }
}
pub struct Secret(String);
impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}
pub struct Login {
    pub session: Secret,
    pub csrf: Secret,
}
impl Login {
    pub fn cookies(&self, c: &Config) -> [String; 2] {
        let secure = if c.secure_cookie { "; Secure" } else { "" };
        let same = if c.strict_cookie { "Strict" } else { "Lax" };
        [
            format!(
                "{}={}; Path=/; HttpOnly{secure}; SameSite={same}; Max-Age={}",
                c.cookie_name(),
                self.session.0,
                c.absolute_seconds
            ),
            format!(
                "{}={}; Path=/{secure}; SameSite={same}; Max-Age={}",
                c.csrf_cookie_name(),
                self.csrf.0,
                c.absolute_seconds
            ),
        ]
    }
}
pub struct Delivery {
    pub email: String,
    pub purpose: &'static str,
    pub token: Secret,
}
#[derive(Clone)]
pub struct Auth {
    pool: Pool,
    config: Config,
    dummy: Arc<String>,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    hash_slots: Arc<tokio::sync::Semaphore>,
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("UTC clock before epoch")
        .as_secs() as i64
}
fn secret() -> Result<Secret> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| Error::Crypto)?;
    Ok(Secret(bytes.iter().map(|b| format!("{b:02x}")).collect()))
}
fn hash(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}
fn valid_secret(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn email(s: &str) -> Result<String> {
    let s = s.trim().to_ascii_lowercase();
    let Some((left, right)) = s.split_once('@') else {
        return Err(Error::InvalidInput);
    };
    if s.len() > 254
        || left.is_empty()
        || right.is_empty()
        || right.contains('@')
        || !s.is_ascii()
        || s.bytes().any(|b| b <= 32 || b == 127)
    {
        return Err(Error::InvalidInput);
    }
    Ok(s)
}
fn password(s: &str) -> Result<()> {
    if !(12..=1024).contains(&s.len()) {
        Err(Error::InvalidInput)
    } else {
        Ok(())
    }
}
fn argon() -> Argon2<'static> {
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(19456, 2, 1, None).unwrap(),
    )
}
fn password_hash(s: &str) -> Result<String> {
    let mut salt = [0; 16];
    getrandom::fill(&mut salt).map_err(|_| Error::Crypto)?;
    let salt = SaltString::encode_b64(&salt).map_err(|_| Error::Crypto)?;
    argon()
        .hash_password(s.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| Error::Crypto)
}
impl Auth {
    /// Inject UTC seconds for deterministic expiry tests or a trusted application clock.
    pub fn with_clock(mut self, clock: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    pub async fn new(pool: Pool, config: Config) -> Result<Self> {
        config.validate()?;
        let dummy = tokio::task::spawn_blocking(|| password_hash("dummy password for equal work"))
            .await
            .map_err(|_| Error::Crypto)??;
        Ok(Self {
            pool,
            config,
            dummy: Arc::new(dummy),
            clock: Arc::new(now),
            hash_slots: Arc::new(tokio::sync::Semaphore::new(2)),
        })
    }
    /// Revalidate and lock the account in the same transaction as a domain mutation.
    pub async fn validate_in(&self, tx: &mut Tx<'_>, principal: &Principal) -> Result<()> {
        if LockUser::query(tx, principal.user_id.to_string(), principal.epoch)
            .await?
            .is_empty()
        {
            return Err(Error::Unauthenticated);
        }
        if principal.is_session() {
            let row = FindSession::query(tx, principal.credential_hash.clone(), true)
                .await?
                .pop()
                .ok_or(Error::Unauthenticated)?;
            let at = (self.clock)();
            if at >= row.expires_at || at - row.seen_at >= self.config.idle_seconds {
                return Err(Error::Unauthenticated);
            }
        } else if FindApi::query(tx, principal.credential_hash.clone(), (self.clock)(), true)
            .await?
            .is_empty()
        {
            return Err(Error::Unauthenticated);
        }
        Ok(())
    }
    pub fn config(&self) -> &Config {
        &self.config
    }
    pub async fn migrate(&self) -> Result<usize> {
        let migrations = aor_migrate::embedded(
            &[("001_auth.sql", include_str!("../migrations/001_auth.sql"))],
            self.pool.dialect(),
        )
        .map_err(|_| Error::Configuration)?;
        aor_migrate::apply(&self.pool, &migrations)
            .await
            .map_err(|_| Error::Configuration)
    }
    async fn hash_password(&self, s: String) -> Result<String> {
        password(&s)?;
        let permit = self
            .hash_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| Error::Crypto)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            password_hash(&s)
        })
        .await
        .map_err(|_| Error::Crypto)?
    }
    /// Returns a delivery only to the trusted mail adapter. HTTP responses must be identical
    /// for new/existing accounts and must never contain delivery secrets.
    pub async fn register(&self, address: &str, plain: String) -> Result<Option<Delivery>> {
        if !self.config.registration_enabled {
            return Err(Error::Forbidden);
        }
        let address = email(address)?;
        let hashed = self.hash_password(plain).await?;
        let id = Uuid::new_v4().to_string();
        let token = secret()?;
        let mut lease = self.pool.acquire().await?;
        let mut tx = lease.begin().await?;
        let inserted =
            InsertUser::execute(&mut tx, id.clone(), address.clone(), hashed, false, 1).await?;
        let delivery = if inserted == 1 {
            InsertOneTime::execute(
                &mut tx,
                hash(&token.0),
                id,
                "verify".into(),
                (self.clock)() + 86400,
            )
            .await?;
            Some(Delivery {
                email: address,
                purpose: "verify",
                token,
            })
        } else {
            None
        };
        tx.commit().await?;
        Ok(delivery)
    }
    pub async fn verify_email(&self, token: &str) -> Result<()> {
        if !valid_secret(token) {
            return Err(Error::Unauthenticated);
        }
        let mut lease = self.pool.acquire().await?;
        let mut tx = lease.begin().await?;
        let row = ConsumeOneTime::query(&mut tx, hash(token), "verify".into(), (self.clock)())
            .await?
            .pop()
            .ok_or(Error::Unauthenticated)?;
        VerifyEmail::execute(&mut tx, true, row.user_id).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn login(&self, address: &str, plain: String, old: Option<&str>) -> Result<Login> {
        if plain.len() > 1024 {
            return Err(Error::Unauthenticated);
        }
        let address = email(address).unwrap_or_else(|_| "invalid@invalid".into());
        let user = {
            let mut lease = self.pool.acquire().await?;
            let mut tx = lease.begin().await?;
            let row = UserByEmail::query(&mut tx, address).await?.pop();
            tx.commit().await?;
            row
        };
        let encoded = user
            .as_ref()
            .map(|u| u.password_hash.clone())
            .unwrap_or_else(|| (*self.dummy).clone());
        let permit = self
            .hash_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| Error::Crypto)?;
        let valid = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            PasswordHash::new(&encoded)
                .ok()
                .is_some_and(|h| argon().verify_password(plain.as_bytes(), &h).is_ok())
        })
        .await
        .map_err(|_| Error::Crypto)?;
        let user = user
            .filter(|u| valid && u.verified)
            .ok_or(Error::Unauthenticated)?;
        let mut lease = self.pool.acquire().await?;
        let mut tx = lease.begin().await?;
        if LockUser::query(&mut tx, user.id.clone(), user.epoch)
            .await?
            .is_empty()
        {
            return Err(Error::Unauthenticated);
        }
        if let Some(old) = old {
            if valid_secret(old) {
                DeleteSession::execute(&mut tx, hash(old)).await?;
            }
        }
        let result = self.create_session(&mut tx, user.id, user.epoch).await?;
        tx.commit().await?;
        Ok(result)
    }
    async fn create_session(&self, tx: &mut Tx<'_>, id: String, epoch: i64) -> Result<Login> {
        let session = secret()?;
        let csrf = secret()?;
        let at = (self.clock)();
        InsertSession::execute(
            tx,
            hash(&session.0),
            id,
            hash(&csrf.0),
            at,
            at,
            at + self.config.absolute_seconds,
            epoch,
        )
        .await?;
        Ok(Login { session, csrf })
    }
    pub async fn authenticate(
        &self,
        headers: &[(String, Vec<u8>)],
        method: &str,
    ) -> Result<Option<Identity>> {
        let cookie = unique_header(headers, "cookie")?;
        let bearer = unique_header(headers, "authorization")?;
        let cookies = parse_cookies(cookie.unwrap_or(""))?;
        let session = cookies.get(self.config.cookie_name());
        if bearer.is_some() && session.is_some() {
            return Err(Error::Unauthenticated);
        }
        if let Some(bearer) = bearer {
            let raw = bearer
                .strip_prefix("Bearer ")
                .filter(|s| valid_secret(s))
                .ok_or(Error::Unauthenticated)?;
            let mut lease = self.pool.acquire().await?;
            let mut tx = lease.begin().await?;
            let row = FindApi::query(&mut tx, hash(raw), (self.clock)(), true)
                .await?
                .pop()
                .ok_or(Error::Unauthenticated)?;
            tx.commit().await?;
            let scopes = serde_json::from_str::<Vec<String>>(&row.scopes)
                .map_err(|_| Error::Unauthenticated)?;
            return Ok(Some(Identity {
                principal: Principal {
                    user_id: Uuid::parse_str(&row.user_id).map_err(|_| Error::Unauthenticated)?,
                    epoch: row.epoch,
                    credential_hash: hash(raw),
                    scopes: Some(scopes),
                },
                session: None,
            }));
        }
        let Some(raw) = session else { return Ok(None) };
        if !valid_secret(raw) {
            return Err(Error::Unauthenticated);
        }
        let digest = hash(raw);
        let mut lease = self.pool.acquire().await?;
        let mut tx = lease.begin().await?;
        let at = (self.clock)();
        let row = FindSession::query(&mut tx, digest.clone(), true)
            .await?
            .pop()
            .ok_or(Error::Unauthenticated)?;
        if at >= row.expires_at
            || at - row.seen_at >= self.config.idle_seconds
            || at < row.created_at
        {
            return Err(Error::Unauthenticated);
        }
        if ["POST", "PUT", "PATCH", "DELETE"].contains(&method) {
            self.check_origin(headers)?;
            let csrf = cookies
                .get(self.config.csrf_cookie_name())
                .ok_or(Error::Csrf)?;
            let supplied = unique_header(headers, "x-csrf-token")?.ok_or(Error::Csrf)?;
            if !valid_secret(csrf)
                || !bool::from(csrf.as_bytes().ct_eq(supplied.as_bytes()))
                || !bool::from(hash(csrf).as_bytes().ct_eq(row.csrf_hash.as_bytes()))
            {
                return Err(Error::Csrf);
            }
        }
        if TouchSession::execute(
            &mut tx,
            at,
            digest.clone(),
            at - self.config.idle_seconds,
            at,
        )
        .await?
            != 1
        {
            return Err(Error::Unauthenticated);
        }
        tx.commit().await?;
        Ok(Some(Identity {
            principal: Principal {
                user_id: Uuid::parse_str(&row.user_id).map_err(|_| Error::Unauthenticated)?,
                epoch: row.epoch,
                credential_hash: digest.clone(),
                scopes: None,
            },
            session: Some(Session {
                token_hash: digest,
                csrf_hash: row.csrf_hash,
            }),
        }))
    }
    pub fn check_origin(&self, headers: &[(String, Vec<u8>)]) -> Result<()> {
        if unique_header(headers, "origin")? != Some(self.config.public_origin.as_str()) {
            return Err(Error::Csrf);
        }
        if unique_header(headers, "sec-fetch-site")?
            .is_some_and(|s| s != "same-origin" && s != "none")
        {
            return Err(Error::Csrf);
        }
        Ok(())
    }
    pub async fn logout(&self, session: &Session) -> Result<()> {
        let mut lease = self.pool.acquire().await?;
        let mut tx = lease.begin().await?;
        DeleteSession::execute(&mut tx, session.token_hash.clone()).await?;
        tx.commit().await?;
        Ok(())
    }
    pub fn clear_cookies(&self) -> [String; 2] {
        let suffix = if self.config.secure_cookie {
            "; Secure"
        } else {
            ""
        };
        [
            format!(
                "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{suffix}",
                self.config.cookie_name()
            ),
            format!(
                "{}=; Path=/; SameSite=Lax; Max-Age=0{suffix}",
                self.config.csrf_cookie_name()
            ),
        ]
    }
    pub async fn revoke_all(&self, principal: &Principal) -> Result<()> {
        if !principal.is_session() {
            return Err(Error::Forbidden);
        }
        let mut lease = self.pool.acquire().await?;
        let mut tx = lease.begin().await?;
        self.validate_in(&mut tx, principal).await?;
        AdvanceEpoch::execute(&mut tx, principal.user_id.to_string()).await?;
        RevokeSessions::execute(&mut tx, principal.user_id.to_string()).await?;
        RevokeApi::execute(&mut tx, principal.user_id.to_string()).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn create_api_token(
        &self,
        principal: &Principal,
        scopes: Vec<String>,
        lifetime: i64,
    ) -> Result<Secret> {
        if !principal.is_session()
            || scopes.is_empty()
            || scopes.len() > 32
            || !(1..=90 * 86400).contains(&lifetime)
            || scopes.iter().any(|s| !valid_scope(s))
        {
            return Err(Error::Forbidden);
        }
        let token = secret()?;
        let mut lease = self.pool.acquire().await?;
        let mut tx = lease.begin().await?;
        if LockUser::query(&mut tx, principal.user_id.to_string(), principal.epoch)
            .await?
            .is_empty()
        {
            return Err(Error::Unauthenticated);
        }
        InsertApi::execute(
            &mut tx,
            hash(&token.0),
            principal.user_id.to_string(),
            serde_json::to_string(&scopes).map_err(|_| Error::InvalidInput)?,
            (self.clock)() + lifetime,
            principal.epoch,
        )
        .await?;
        tx.commit().await?;
        Ok(token)
    }
    pub async fn revoke_api_token(&self, principal: &Principal, raw: &str) -> Result<()> {
        if !principal.is_session() {
            return Err(Error::Forbidden);
        }
        let mut lease = self.pool.acquire().await?;
        let mut tx = lease.begin().await?;
        self.validate_in(&mut tx, principal).await?;
        DeleteApi::execute(&mut tx, hash(raw), principal.user_id.to_string()).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn request_reset(&self, address: &str) -> Result<Option<Delivery>> {
        let address = email(address).unwrap_or_else(|_| "invalid@invalid".into());
        let token = secret()?;
        let mut lease = self.pool.acquire().await?;
        let mut tx = lease.begin().await?;
        let user = UserByEmail::query(&mut tx, address.clone()).await?.pop();
        let result = if let Some(user) = user {
            InsertOneTime::execute(
                &mut tx,
                hash(&token.0),
                user.id,
                "reset".into(),
                (self.clock)() + 1800,
            )
            .await?;
            Some(Delivery {
                email: address,
                purpose: "reset",
                token,
            })
        } else {
            None
        };
        tx.commit().await?;
        Ok(result)
    }
    pub async fn reset_password(&self, token: &str, plain: String) -> Result<()> {
        if !valid_secret(token) {
            return Err(Error::Unauthenticated);
        }
        let encoded = self.hash_password(plain).await?;
        let mut lease = self.pool.acquire().await?;
        let mut tx = lease.begin().await?;
        let user = ConsumeOneTime::query(&mut tx, hash(token), "reset".into(), (self.clock)())
            .await?
            .pop()
            .ok_or(Error::Unauthenticated)?;
        ResetPassword::execute(&mut tx, encoded, user.user_id.clone()).await?;
        RevokeSessions::execute(&mut tx, user.user_id.clone()).await?;
        RevokeApi::execute(&mut tx, user.user_id.clone()).await?;
        RevokeOneTime::execute(&mut tx, user.user_id).await?;
        tx.commit().await?;
        Ok(())
    }
}
fn valid_scope(s: &str) -> bool {
    let Some((resource, action)) = s.split_once(':') else {
        return false;
    };
    !resource.is_empty()
        && resource.len() <= 64
        && resource
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b == b'_')
        && ["read", "create", "update", "delete"].contains(&action)
}
pub fn unique_header<'a>(headers: &'a [(String, Vec<u8>)], name: &str) -> Result<Option<&'a str>> {
    let mut values = headers.iter().filter(|(n, _)| n.eq_ignore_ascii_case(name));
    let value = values
        .next()
        .map(|(_, v)| std::str::from_utf8(v).map_err(|_| Error::Unauthenticated))
        .transpose()?;
    if values.next().is_some() {
        return Err(Error::Unauthenticated);
    }
    Ok(value)
}
pub fn parse_cookies(header: &str) -> Result<BTreeMap<String, String>> {
    let mut cookies = BTreeMap::new();
    if header.is_empty() {
        return Ok(cookies);
    }
    for item in header.split(';') {
        let Some((name, value)) = item.trim().split_once('=') else {
            return Err(Error::Unauthenticated);
        };
        if name.is_empty()
            || cookies.len() >= 64
            || cookies.insert(name.into(), value.into()).is_some()
        {
            return Err(Error::Unauthenticated);
        }
    }
    Ok(cookies)
}
impl Session {
    pub fn verify_csrf(&self, token: &str) -> bool {
        valid_secret(token) && bool::from(hash(token).as_bytes().ct_eq(self.csrf_hash.as_bytes()))
    }
}
