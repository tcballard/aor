use aor_db::{Pool, PoolOptions, Uuid};
use aor_http::{Body, Request};
use aor_jobs::{Idempotent, Job};
use aor_policy::{Read, Update};
use aor_registry::{
    State,
    manifests::{self, ValidateManifestV1},
    versions::Version,
};
use aor_session::{Auth, Config, Login};
use serde_json::{Value, json};
use std::sync::Arc;
fn config() -> Config {
    Config {
        public_origin: "http://localhost:3001".into(),
        production: false,
        secure_cookie: false,
        strict_cookie: false,
        idle_seconds: 1800,
        absolute_seconds: 86400,
        registration_enabled: true,
    }
}
async fn account(auth: &Auth, email: &str) -> Login {
    let delivery = auth
        .register(email, "a long test password".into())
        .await
        .unwrap()
        .unwrap();
    auth.verify_email(delivery.token.expose()).await.unwrap();
    auth.login(email, "a long test password".into(), None)
        .await
        .unwrap()
}
async fn identity(auth: &Auth, login: &Login) -> aor_session::Identity {
    auth.authenticate(
        &[(
            "Cookie".into(),
            format!("aor_dev_session={}", login.session.expose()).into_bytes(),
        )],
        "GET",
    )
    .await
    .unwrap()
    .unwrap()
}
fn manifest() -> String {
    json!({"schemaVersion":1,"id":"io.example.plugin","name":"Plugin","version":"1.0.0","kinds":["menu"],"entryPoints":{"menu":"Menu.qml"}}).to_string()
}
async fn draft(state: &State, identity: &aor_session::Identity) -> Uuid {
    let p = aor_registry::plugins::create(
        state,
        aor_policy::owner(Some(identity.principal())).unwrap(),
        aor_registry::plugins::Input {
            name: "Plugin".into(),
        },
    )
    .await
    .unwrap();
    aor_registry::versions::create(
        state,
        aor_policy::owner(Some(identity.principal())).unwrap(),
        aor_registry::versions::Input {
            name: "1.0.0".into(),
            plugin_id: p.id,
        },
    )
    .await
    .unwrap()
    .id
}
async fn submit(
    state: &State,
    identity: &aor_session::Identity,
    id: Uuid,
    body: String,
) -> Result<manifests::Output, aor_router::AppError> {
    manifests::submit(
        state,
        aor_policy::owner::<Version, Update>(Some(identity.principal())).unwrap(),
        id,
        manifests::Upload {
            version: 1,
            manifest: body,
        },
    )
    .await
}
async fn outcome(state: &State, identity: &aor_session::Identity, id: Uuid) -> manifests::Output {
    manifests::get(
        state,
        aor_policy::owner::<Version, Read>(Some(identity.principal())).unwrap(),
        id,
    )
    .await
    .unwrap()
}
aor_db::sql!(
    CountPublications,
    portable,
    "migrations",
    "SELECT COUNT(version_id) AS total FROM publications WHERE version_id = $1"
);
aor_db::sql!(
    CountUploads,
    portable,
    "migrations",
    "SELECT COUNT(version_id) AS total FROM manifest_uploads WHERE version_id = $1"
);
async fn suite(pool: Pool) {
    aor_registry::migrate(&pool).await.unwrap();
    let auth = Auth::new(pool.clone(), config()).await.unwrap();
    let alice = account(&auth, "alice@publication.test").await;
    let bob = account(&auth, "bob@publication.test").await;
    let alice_id = identity(&auth, &alice).await;
    let bob_id = identity(&auth, &bob).await;
    let state = State {
        pool: pool.clone(),
        auth: auth.clone(),
        mail_spool: None,
    };
    let id = draft(&state, &alice_id).await;
    assert_eq!(
        submit(&state, &bob_id, id, manifest()).await.err(),
        Some(aor_router::AppError::NotFound)
    );
    let router = Arc::new(
        aor_registry::router(
            pool.clone(),
            Config {
                registration_enabled: false,
                ..config()
            },
            None,
        )
        .await
        .unwrap(),
    );
    for (login, csrf, status) in [
        (None, false, 401),
        (Some(&alice), false, 403),
        (Some(&bob), true, 404),
    ] {
        let mut headers = vec![("Content-Type".into(), b"application/json".to_vec())];
        if let Some(login) = login {
            headers.push((
                "Cookie".into(),
                format!(
                    "aor_dev_session={}; aor_dev_csrf={}",
                    login.session.expose(),
                    login.csrf.expose()
                )
                .into_bytes(),
            ));
            if csrf {
                headers.push(("Origin".into(), b"http://localhost:3001".to_vec()));
                headers.push((
                    "X-CSRF-Token".into(),
                    login.csrf.expose().as_bytes().to_vec(),
                ));
            }
        }
        let response = router
            .handle(Request {
                method: "POST".into(),
                target: format!("/versions/{id}/manifest"),
                headers,
                body: Body::from_bytes(
                    json!({"version":1,"manifest":manifest()})
                        .to_string()
                        .into_bytes(),
                    131072,
                )
                .unwrap(),
            })
            .await;
        assert_eq!(response.status, status);
    }
    assert!(
        submit(&state, &alice_id, id, "x".repeat(65537))
            .await
            .is_err()
    );
    assert_eq!(
        submit(&state, &alice_id, id, manifest())
            .await
            .unwrap()
            .status,
        "pending"
    );
    assert_eq!(
        submit(&state, &alice_id, id, manifest()).await.err(),
        Some(aor_router::AppError::Conflict)
    );
    assert_eq!(
        aor_registry::versions::update(
            &state,
            aor_policy::owner(Some(alice_id.principal())).unwrap(),
            id,
            aor_registry::versions::Change {
                name: "2.0.0".into(),
                version: 1
            }
        )
        .await
        .err(),
        Some(aor_router::AppError::Conflict)
    );
    let at = aor_db::Utc::now().timestamp();
    let old = aor_jobs::claim(&pool, "registry", at, 1)
        .await
        .unwrap()
        .unwrap();
    let new = aor_jobs::claim(&pool, "registry", at + 1, 30)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        manifests::process(&pool, &old, at + 1).await,
        Err(aor_jobs::Error::Stale)
    ));
    assert_eq!(outcome(&state, &alice_id, id).await.status, "pending");
    manifests::process(&pool, &new, at + 2).await.unwrap();
    assert!(matches!(
        manifests::process(&pool, &new, at + 2).await,
        Err(aor_jobs::Error::Stale)
    ));
    assert_eq!(outcome(&state, &alice_id, id).await.status, "published");
    {
        let mut lease = pool.acquire().await.unwrap();
        assert_eq!(
            CountPublications::query(&mut lease, id.to_string())
                .await
                .unwrap()[0]
                .total,
            1
        );
    }
    let rejected = draft(&state, &alice_id).await;
    submit(&state, &alice_id, rejected, "{\"schemaVersion\":99}".into())
        .await
        .unwrap();
    manifests::work_once(&pool, at + 3).await.unwrap();
    let result = outcome(&state, &alice_id, rejected).await;
    assert_eq!(result.status, "rejected");
    assert!(result.findings.contains(&"AOR-MANIFEST-SCHEMA".into()));
    let rolled = draft(&state, &alice_id).await;
    {
        let mut lease = pool.acquire().await.unwrap();
        let mut tx = lease.begin().await.unwrap();
        aor_db::sql!(
            InsertUpload,
            portable,
            "migrations",
            "INSERT INTO manifest_uploads (version_id, revision, manifest, status, findings, submitted_at) VALUES ($1, 1, $2, 'pending', '[]', $3)"
        );
        InsertUpload::execute(&mut tx, rolled.to_string(), manifest(), at)
            .await
            .unwrap();
        aor_jobs::enqueue_in(
            &mut tx,
            &ValidateManifestV1 {
                version_id: rolled.to_string(),
                revision: 1,
            },
            "registry",
            at,
            aor_jobs::Options::default(),
        )
        .await
        .unwrap();
        tx.rollback().await.unwrap();
    }
    assert!(!manifests::work_once(&pool, at + 4).await.unwrap());
    {
        let mut lease = pool.acquire().await.unwrap();
        assert_eq!(
            CountUploads::query(&mut lease, rolled.to_string())
                .await
                .unwrap()[0]
                .total,
            0
        );
    }
    pool.close();
}
#[tokio::test]
async fn sqlite_publication_matrix() {
    let file = std::env::temp_dir().join(format!("aor-publication-{}.sqlite", Uuid::new_v4()));
    suite(Pool::sqlite(&file, PoolOptions::default()).unwrap()).await;
    std::fs::remove_file(file).unwrap();
}
#[tokio::test]
#[ignore = "requires isolated AOR_TEST_DATABASE_URL"]
async fn postgres_publication_matrix() {
    suite(
        Pool::postgres(
            &std::env::var("AOR_TEST_DATABASE_URL").unwrap(),
            PoolOptions::default(),
        )
        .unwrap(),
    )
    .await;
}
#[test]
fn manifest_denials() {
    assert!(manifests::validate(&manifest(), "1.0.0").is_empty());
    for (field, value) in [
        ("id", json!("omarchy.reserved")),
        ("id", json!("../escape")),
        ("version", json!("2.0.0")),
        ("schemaVersion", json!("1")),
        ("kinds", json!(["unknown"])),
        ("entryPoints", json!({"menu":"../Menu.qml"})),
        ("entryPoints", json!({"menu":"/Menu.qml"})),
        ("entryPoints", json!({"menu":"x\n.qml"})),
        ("entryPoints", json!({})),
        ("name", Value::Null),
    ] {
        let mut m: Value = serde_json::from_str(&manifest()).unwrap();
        m[field] = value;
        assert!(
            !manifests::validate(&m.to_string(), "1.0.0").is_empty(),
            "{field}"
        );
    }
}
#[tokio::test]
async fn killed_worker_recovers_one_publication() {
    let file = std::env::temp_dir().join(format!("aor-kill-{}.sqlite", Uuid::new_v4()));
    let ready = file.with_extension("ready");
    let pool = Pool::sqlite(&file, PoolOptions::default()).unwrap();
    aor_registry::migrate(&pool).await.unwrap();
    let auth = Auth::new(pool.clone(), config()).await.unwrap();
    let login = account(&auth, "kill@publication.test").await;
    let identity = identity(&auth, &login).await;
    let state = State {
        pool: pool.clone(),
        auth,
        mail_spool: None,
    };
    let id = draft(&state, &identity).await;
    submit(&state, &identity, id, manifest()).await.unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_worker", "--nocapture"])
        .env("AOR_CHILD_DATABASE", &file)
        .env("AOR_CHILD_READY", &ready)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    for _ in 0..100 {
        if ready.exists() {
            break;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("child exited before claim: {status}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    if !ready.exists() {
        child.kill().unwrap();
        child.wait().unwrap();
        panic!("child did not claim");
    }
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
    let job_id = std::fs::read_to_string(&ready).unwrap();
    assert_eq!(
        aor_jobs::inspect(&pool, &job_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        "running"
    );
    assert_eq!(outcome(&state, &identity, id).await.status, "pending");
    let recovery = aor_db::Utc::now().timestamp() + 31;
    manifests::work_once(&pool, recovery).await.unwrap();
    assert!(!manifests::work_once(&pool, recovery + 1).await.unwrap());
    assert_eq!(outcome(&state, &identity, id).await.status, "published");
    let mut lease = pool.acquire().await.unwrap();
    assert_eq!(
        CountPublications::query(&mut lease, id.to_string())
            .await
            .unwrap()[0]
            .total,
        1
    );
    drop(lease);
    pool.close();
    std::fs::remove_file(file).unwrap();
    std::fs::remove_file(ready).unwrap();
}
#[tokio::test]
async fn child_worker() {
    let Some(file) = std::env::var_os("AOR_CHILD_DATABASE") else {
        return;
    };
    let pool = Pool::sqlite(std::path::PathBuf::from(file), PoolOptions::default()).unwrap();
    let claim = aor_jobs::claim(&pool, "registry", aor_db::Utc::now().timestamp(), 30)
        .await
        .unwrap()
        .unwrap();
    std::fs::write(std::env::var_os("AOR_CHILD_READY").unwrap(), claim.id()).unwrap();
    std::future::pending::<()>().await;
}
#[derive(serde::Serialize, serde::Deserialize)]
struct FutureJob;
impl Job for FutureJob {
    const NAME: &'static str = "registry.validate-manifest";
    const VERSION: i64 = 999;
    const IDEMPOTENCY: Idempotent = Idempotent::Natural;
    fn key(&self) -> String {
        "future".into()
    }
}
#[tokio::test]
async fn rolling_deploy_parks_unknown_payload_without_burning_attempts() {
    let file = std::env::temp_dir().join(format!("aor-future-{}.sqlite", Uuid::new_v4()));
    let pool = Pool::sqlite(&file, PoolOptions::default()).unwrap();
    aor_registry::migrate(&pool).await.unwrap();
    let mut lease = pool.acquire().await.unwrap();
    let mut tx = lease.begin().await.unwrap();
    let id = aor_jobs::enqueue_in(
        &mut tx,
        &FutureJob,
        "registry",
        100,
        aor_jobs::Options::default(),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    drop(lease);
    manifests::work_once(&pool, 100).await.unwrap();
    let row = aor_jobs::inspect(&pool, &id).await.unwrap().unwrap();
    assert_eq!(row.state, "incompatible");
    assert_eq!(row.attempt, 0);
    pool.close();
    std::fs::remove_file(file).unwrap();
}
