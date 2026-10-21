mod accounts;
pub mod manifests;
pub mod plugins;
pub mod versions;
pub mod worker;
use aor_db::{Dialect, Pool};
use aor_http::Response;
use aor_router::{AppError, Router};
use aor_session::{Auth, Config};
use std::{path::PathBuf, sync::Arc};
#[derive(Clone)]
pub struct State {
    pub pool: Pool,
    pub auth: Auth,
    pub mail_spool: Option<PathBuf>,
}
pub async fn migrate(pool: &Pool) -> Result<usize, Box<dyn std::error::Error>> {
    let migrations = aor_migrate::embedded(
        &[
            ("001_auth.sql", include_str!("../migrations/001_auth.sql")),
            (
                "002_plugins.sql",
                include_str!("../migrations/002_plugins.sql"),
            ),
            (
                "003_versions.sql",
                include_str!("../migrations/003_versions.sql"),
            ),
            ("004_jobs.sql", include_str!("../migrations/004_jobs.sql")),
            (
                "005_uploads.sql",
                include_str!("../migrations/005_uploads.sql"),
            ),
            ("006_mail.sql", include_str!("../migrations/006_mail.sql")),
        ],
        pool.dialect(),
    )?;
    Ok(aor_migrate::apply(pool, &migrations).await?)
}
pub async fn router(
    pool: Pool,
    config: Config,
    mail_spool: Option<PathBuf>,
) -> Result<Router, Box<dyn std::error::Error>> {
    router_with_outbox(pool, config, mail_spool, None).await
}
pub async fn router_with_outbox(
    pool: Pool,
    config: Config,
    mail_spool: Option<PathBuf>,
    outbox: Option<aor_mail::Outbox>,
) -> Result<Router, Box<dyn std::error::Error>> {
    if outbox.is_some() && mail_spool.is_some() {
        return Err("choose queued mail or the development spool".into());
    }
    if config.production && outbox.is_none() {
        return Err("production accounts require a durable mail outbox".into());
    }
    if config.production && mail_spool.is_some() {
        return Err("development mail spool is forbidden in production".into());
    }
    if config.registration_enabled && mail_spool.is_none() && outbox.is_none() {
        return Err("registration requires a configured mail delivery adapter".into());
    }
    let mut auth = Auth::new(pool.clone(), config).await?;
    if let Some(outbox) = outbox {
        auth = auth.with_outbox(outbox);
    }
    let state = Arc::new(State {
        pool,
        auth: auth.clone(),
        mail_spool,
    });
    let mut routes = accounts::routes(state.clone());
    routes.extend(plugins::routes(state.clone()));
    routes.extend(versions::routes(state.clone()));
    routes.extend(manifests::routes(state));
    Ok(Router::new(routes)?.with_auth(auth))
}
pub fn json<T: serde::Serialize>(status: u16, value: T) -> Result<Response, AppError> {
    Response::new(
        status,
        serde_json::to_vec(&value).map_err(|_| AppError::Internal)?,
    )
    .header("Content-Type", "application/json")
    .map_err(|_| AppError::Internal)
}
fn db_error(_: aor_db::Error) -> AppError {
    AppError::Internal
}
fn policy_error(e: aor_policy::Denied) -> AppError {
    match e {
        aor_policy::Denied::Unauthenticated => AppError::Unauthenticated,
        aor_policy::Denied::NotFound => AppError::NotFound,
        aor_policy::Denied::Forbidden => AppError::Forbidden,
    }
}
pub fn dialect_name(pool: &Pool) -> &'static str {
    match pool.dialect() {
        Dialect::Postgres => "postgres",
        Dialect::Sqlite => "sqlite",
    }
}
