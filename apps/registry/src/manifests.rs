use crate::{State, db_error, policy_error, versions::Version};
use aor_db::{Pool, Uuid};
use aor_jobs::{Claim, Idempotent, Job};
use aor_policy::{Authorized, Read, Update};
use aor_router::{AppError, Route};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Upload {
    pub version: i64,
    pub manifest: String,
}
#[derive(Serialize)]
pub struct Output {
    pub version_id: String,
    pub status: String,
    pub findings: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidateManifestV1 {
    pub version_id: String,
    pub revision: i64,
}
impl Job for ValidateManifestV1 {
    const NAME: &'static str = "registry.validate-manifest";
    const VERSION: i64 = 1;
    const IDEMPOTENCY: Idempotent = Idempotent::Natural;
    fn key(&self) -> String {
        format!("{}:{}", self.version_id, self.revision)
    }
}
aor_db::sql!(
    LockVersion,
    portable,
    "migrations",
    "UPDATE versions SET version = version WHERE id = $1 AND owner_id = $2 AND version = $3 RETURNING name"
);
aor_db::sql!(
    FindVersion,
    portable,
    "migrations",
    "SELECT id FROM versions WHERE id = $1 AND owner_id = $2"
);
aor_db::sql!(
    UploadRow,
    portable,
    "migrations",
    "INSERT INTO manifest_uploads (version_id, revision, manifest, status, findings, submitted_at) VALUES ($1, $2, $3, 'pending', '[]', $4) ON CONFLICT (version_id) DO NOTHING"
);
aor_db::sql!(
    ReadUpload,
    portable,
    "migrations",
    "SELECT u.version_id, u.status, u.findings FROM manifest_uploads AS u INNER JOIN versions AS v ON v.id = u.version_id WHERE v.id = $1 AND v.owner_id = $2"
);
aor_db::sql!(
    Pending,
    portable,
    "migrations",
    "SELECT u.manifest, u.revision, u.status, v.name, v.version FROM manifest_uploads AS u INNER JOIN versions AS v ON v.id = u.version_id WHERE u.version_id = $1"
);
aor_db::sql!(
    Publish,
    portable,
    "migrations",
    "INSERT INTO publications (version_id, revision, published_at) VALUES ($1, $2, $3) ON CONFLICT (version_id) DO NOTHING"
);
aor_db::sql!(
    Outcome,
    portable,
    "migrations",
    "UPDATE manifest_uploads SET status = $1, findings = $2 WHERE version_id = $3 AND revision = $4 AND status = 'pending'"
);
fn now() -> i64 {
    aor_db::Utc::now().timestamp()
}
pub async fn submit(
    state: &State,
    scope: Authorized<Version, Update>,
    id: Uuid,
    input: Upload,
) -> Result<Output, AppError> {
    if input.version < 1 || input.manifest.len() > 65536 || input.manifest.is_empty() {
        return Err(AppError::BadJson);
    }
    let mut lease = state.pool.acquire().await.map_err(db_error)?;
    let mut tx = aor_tx::begin(&mut lease, &state.auth, &scope)
        .await
        .map_err(aor_router::auth_error)?;
    if FindVersion::query(&mut tx, id.to_string(), scope.owner_id().to_string())
        .await
        .map_err(db_error)?
        .is_empty()
    {
        return Err(AppError::NotFound);
    }
    if LockVersion::query(
        &mut tx,
        id.to_string(),
        scope.owner_id().to_string(),
        input.version,
    )
    .await
    .map_err(db_error)?
    .is_empty()
    {
        return Err(AppError::Conflict);
    }
    let at = now();
    if UploadRow::execute(&mut tx, id.to_string(), input.version, input.manifest, at)
        .await
        .map_err(db_error)?
        != 1
    {
        return Err(AppError::Conflict);
    }
    aor_jobs::enqueue_in(
        &mut tx,
        &ValidateManifestV1 {
            version_id: id.to_string(),
            revision: input.version,
        },
        "registry",
        at,
        aor_jobs::Options::default(),
    )
    .await
    .map_err(|_| AppError::Internal)?;
    tx.commit().await.map_err(db_error)?;
    Ok(Output {
        version_id: id.to_string(),
        status: "pending".into(),
        findings: vec![],
    })
}
pub async fn get(
    state: &State,
    scope: Authorized<Version, Read>,
    id: Uuid,
) -> Result<Output, AppError> {
    let mut lease = state.pool.acquire().await.map_err(db_error)?;
    let mut tx = aor_tx::begin(&mut lease, &state.auth, &scope)
        .await
        .map_err(aor_router::auth_error)?;
    let row = ReadUpload::query(&mut tx, id.to_string(), scope.owner_id().to_string())
        .await
        .map_err(db_error)?
        .pop()
        .ok_or(AppError::NotFound)?;
    tx.commit().await.map_err(db_error)?;
    Ok(Output {
        version_id: row.version_id,
        status: row.status,
        findings: serde_json::from_str(&row.findings).map_err(|_| AppError::Internal)?,
    })
}
fn text(value: Option<&serde_json::Value>, max: usize) -> bool {
    value
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control))
}
fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 256
        && !path.starts_with('/')
        && !path.contains("..")
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && path.split('/').all(|p| !p.is_empty() && p != ".")
}
pub fn validate(bytes: &str, version: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(bytes) else {
        return vec!["AOR-MANIFEST-JSON".into()];
    };
    let Some(m) = value.as_object() else {
        return vec!["AOR-MANIFEST-OBJECT".into()];
    };
    let mut findings = Vec::new();
    let mut check = |pass: bool, code: &str| {
        if !pass {
            findings.push(code.into())
        }
    };
    check(
        m.get("schemaVersion").and_then(|v| v.as_u64()) == Some(1),
        "AOR-MANIFEST-SCHEMA",
    );
    let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("");
    check(
        !id.is_empty()
            && id.len() <= 128
            && id.as_bytes()[0].is_ascii_alphanumeric()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            && !id.contains("..")
            && !id.starts_with("omarchy."),
        "AOR-MANIFEST-ID",
    );
    check(text(m.get("name"), 128), "AOR-MANIFEST-NAME");
    check(
        m.get("version")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s == version && s.parse::<aor_router::SemVer>().is_ok()),
        "AOR-MANIFEST-VERSION",
    );
    let entries = m.get("entryPoints").and_then(|v| v.as_object());
    check(
        entries.is_some_and(|e| {
            e.len() <= 16 && e.values().all(|v| v.as_str().is_some_and(safe_path))
        }),
        "AOR-MANIFEST-PATH",
    );
    let kinds = m.get("kinds").and_then(|v| v.as_array());
    let known = [
        ("bar", "bar"),
        ("bar-widget", "barWidget"),
        ("menu", "menu"),
        ("overlay", "overlay"),
        ("panel", "panel"),
        ("service", "service"),
    ];
    check(
        kinds.is_some_and(|k| {
            !k.is_empty()
                && k.len() <= 6
                && k.iter().all(|kind| {
                    kind.as_str().is_some_and(|s| {
                        known.iter().any(|(name, key)| {
                            s == *name && entries.is_some_and(|e| e.contains_key(*key))
                        })
                    })
                })
        }),
        "AOR-MANIFEST-KINDS",
    );
    if let Some(section) = m.get("barWidget").and_then(|v| v.get("defaultSection")) {
        check(
            section.as_str().is_some_and(|s| {
                [
                    "left",
                    "centerBegin",
                    "center",
                    "centerEnd",
                    "tray",
                    "right",
                ]
                .contains(&s)
            }),
            "AOR-MANIFEST-SECTION",
        );
    }
    findings
}
pub async fn process(pool: &Pool, claim: &Claim, at: i64) -> aor_jobs::Result<()> {
    let job = claim.decode::<ValidateManifestV1>()?;
    let mut lease = pool.acquire().await?;
    let mut tx = lease.begin().await?;
    aor_jobs::fence_in(&mut tx, claim, at).await?;
    if let Some(row) = Pending::query(&mut tx, job.version_id.clone()).await?.pop() {
        if row.status == "pending" && row.revision == job.revision {
            let findings = if row.version == job.revision {
                validate(&row.manifest, &row.name)
            } else {
                vec!["AOR-MANIFEST-STALE".into()]
            };
            let status = if findings.is_empty() {
                "published"
            } else {
                "rejected"
            };
            if findings.is_empty() {
                Publish::execute(&mut tx, job.version_id.clone(), job.revision, at).await?;
            }
            Outcome::execute(
                &mut tx,
                status.into(),
                serde_json::to_string(&findings).map_err(|_| aor_jobs::Error::Invalid)?,
                job.version_id,
                job.revision,
            )
            .await?;
        }
    }
    aor_jobs::complete_in(&mut tx, claim, at).await?;
    tx.commit().await?;
    Ok(())
}
pub async fn work_once(pool: &Pool, at: i64) -> aor_jobs::Result<bool> {
    let Some(claim) = aor_jobs::claim(pool, "registry", at, 30).await? else {
        return Ok(false);
    };
    if claim.job_type() != ValidateManifestV1::NAME
        || claim.payload_version() != ValidateManifestV1::VERSION
    {
        aor_jobs::incompatible(pool, &claim, at).await?;
    } else if let Err(error) = process(pool, &claim, at).await {
        if matches!(error, aor_jobs::Error::Stale) {
            return Err(error);
        }
        aor_jobs::fail(pool, &claim, now(), aor_jobs::Failure::Handler).await?;
    }
    Ok(true)
}
pub fn routes(state: Arc<State>) -> Vec<Route> {
    let submit_state = state.clone();
    vec![
        Route::protected(
            "POST",
            "/versions/{id}/manifest",
            "manifests::submit",
            "owner<Version,Update>",
            move |ctx| {
                let state = submit_state.clone();
                async move {
                    let scope = aor_policy::owner::<Version, Update>(ctx.principal())
                        .map_err(policy_error)?;
                    let id = ctx.path.get::<Uuid>("id")?;
                    let out = submit(&state, scope, id, ctx.json().await?).await?;
                    crate::json(202, aor_policy::View(out))
                }
            },
        ),
        Route::protected(
            "GET",
            "/versions/{id}/manifest",
            "manifests::get",
            "owner<Version,Read>",
            move |ctx| {
                let state = state.clone();
                async move {
                    let scope = aor_policy::owner::<Version, Read>(ctx.principal())
                        .map_err(policy_error)?;
                    let id = ctx.path.get::<Uuid>("id")?;
                    let out = get(&state, scope, id).await?;
                    crate::json(200, aor_policy::View(out))
                }
            },
        ),
    ]
}
