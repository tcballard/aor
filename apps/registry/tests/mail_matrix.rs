use aor_db::{Pool, PoolOptions, Uuid};
use aor_mail::{Key, Message, Outbox, Transport, Worker};
use aor_session::{Auth, Config};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicI64, Ordering},
};
#[derive(Clone, Default)]
struct Provider {
    deliveries: Arc<Mutex<std::collections::BTreeMap<String, String>>>,
    attempts: Arc<Mutex<Vec<String>>>,
    lose_response: Arc<AtomicBool>,
}
impl Transport for Provider {
    async fn send(&self, m: &Message) -> aor_mail::Result<()> {
        assert_eq!(m.sender(), "registry@example.test");
        assert!(m.recipient().ends_with("@example.test"));
        self.attempts
            .lock()
            .unwrap()
            .push(m.idempotency_key().into());
        let mut deliveries = self.deliveries.lock().unwrap();
        if let Some(old) = deliveries.get(m.idempotency_key()) {
            assert_eq!(old, m.text());
        } else {
            deliveries.insert(m.idempotency_key().into(), m.text().into());
        }
        if self.lose_response.swap(false, Ordering::SeqCst) {
            return Err(aor_mail::Error::Transport);
        }
        Ok(())
    }
}
fn config() -> Config {
    Config {
        public_origin: "https://registry.example.test".into(),
        production: true,
        secure_cookie: true,
        strict_cookie: false,
        idle_seconds: 60,
        absolute_seconds: 3600,
        registration_enabled: true,
    }
}
fn outbox() -> Outbox {
    Outbox::new(Key::new([42; 32]).unwrap(), "registry@example.test".into()).unwrap()
}
async fn auth(pool: &Pool, clock: &Arc<AtomicI64>) -> Auth {
    let c = clock.clone();
    Auth::new(pool.clone(), config())
        .await
        .unwrap()
        .with_clock(move || c.load(Ordering::SeqCst))
        .with_outbox(outbox())
}
fn worker(pool: &Pool, provider: Provider, clock: &Arc<AtomicI64>) -> Worker<Provider> {
    let c = clock.clone();
    Worker::new(pool.clone(), Key::new([42; 32]).unwrap(), provider)
        .with_clock(move || c.load(Ordering::SeqCst))
}
fn code(provider: &Provider, index: usize) -> String {
    let attempts = provider.attempts.lock().unwrap();
    let key = &attempts[index];
    provider.deliveries.lock().unwrap()[key]
        .lines()
        .nth(2)
        .unwrap()
        .into()
}
aor_db::sql!(
    Counts,
    portable,
    "migrations",
    "SELECT COUNT(id) AS total FROM aor_mail"
);
aor_db::sql!(
    Users,
    portable,
    "migrations",
    "SELECT COUNT(id) AS total FROM aor_users"
);
aor_db::sql!(
    StoredPayloads,
    portable,
    "migrations",
    "SELECT payload FROM aor_jobs WHERE queue = 'mail'"
);
aor_db::sql!(
    States,
    portable,
    "migrations",
    "SELECT state, token_digest FROM aor_mail ORDER BY created_at, id"
);
async fn suite(pool: Pool) {
    aor_registry::migrate(&pool).await.unwrap();
    let clock = Arc::new(AtomicI64::new(1000));
    let auth = auth(&pool, &clock).await;
    let provider = Provider::default();
    let worker = worker(&pool, provider.clone(), &clock);
    assert!(
        auth.register("alice@example.test", "first strong password".into())
            .await
            .unwrap()
            .is_none()
    );
    assert!(provider.deliveries.lock().unwrap().is_empty());
    assert!(
        auth.login("alice@example.test", "first strong password".into(), None)
            .await
            .is_err()
    );
    assert!(worker.work_once().await.unwrap());
    let verify = code(&provider, 0);
    assert_eq!(verify.len(), 64);
    auth.verify_email(&verify).await.unwrap();
    assert!(auth.verify_email(&verify).await.is_err());
    auth.login("alice@example.test", "first strong password".into(), None)
        .await
        .unwrap();
    auth.request_reset("missing@example.test").await.unwrap();
    assert!(!worker.work_once().await.unwrap());
    assert!(
        auth.request_reset("alice@example.test")
            .await
            .unwrap()
            .is_none()
    );
    provider.lose_response.store(true, Ordering::SeqCst);
    assert!(matches!(
        worker.work_once().await,
        Err(aor_mail::Error::Transport)
    ));
    let reset = code(&provider, 1);
    assert_ne!(reset, verify);
    clock.store(1100, Ordering::SeqCst);
    assert!(worker.work_once().await.unwrap());
    assert_eq!(provider.deliveries.lock().unwrap().len(), 2);
    assert_eq!(provider.attempts.lock().unwrap().len(), 3);
    auth.reset_password(&reset, "replacement strong password".into())
        .await
        .unwrap();
    assert!(
        auth.reset_password(&reset, "replacement strong password".into())
            .await
            .is_err()
    );
    auth.login(
        "alice@example.test",
        "replacement strong password".into(),
        None,
    )
    .await
    .unwrap();
    let mut lease = pool.acquire().await.unwrap();
    for row in StoredPayloads::query(&mut lease).await.unwrap() {
        assert!(!row.payload.contains(&verify));
        assert!(!row.payload.contains(&reset));
        assert!(!row.payload.contains("alice@"));
        let value: serde_json::Value = serde_json::from_str(&row.payload).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 1);
        assert!(value.get("delivery_id").is_some());
    }
    for row in States::query(&mut lease).await.unwrap() {
        assert_eq!(row.state, "delivered");
        assert_ne!(row.token_digest, verify);
        assert_ne!(row.token_digest, reset);
    }
    drop(lease);
    pool.close();
}
#[tokio::test]
async fn sqlite_atomic_mail_and_replay() {
    let file = std::env::temp_dir().join(format!("aor-mail-matrix-{}.sqlite", Uuid::new_v4()));
    suite(Pool::sqlite(&file, PoolOptions::default()).unwrap()).await;
    std::fs::remove_file(file).unwrap();
}
#[tokio::test]
#[ignore = "requires isolated AOR_TEST_DATABASE_URL"]
async fn postgres_atomic_mail_and_replay() {
    suite(
        Pool::postgres(
            &std::env::var("AOR_TEST_DATABASE_URL").unwrap(),
            PoolOptions::default(),
        )
        .unwrap(),
    )
    .await;
}
#[tokio::test]
async fn producer_failure_rolls_back_account_outbox_and_token() {
    let file = std::env::temp_dir().join(format!("aor-mail-rollback-{}.sqlite", Uuid::new_v4()));
    let pool = Pool::sqlite(&file, PoolOptions::default()).unwrap();
    aor_registry::migrate(&pool).await.unwrap();
    let clock = Arc::new(AtomicI64::new(1000));
    let auth = auth(&pool, &clock).await;
    {
        let mut lease = pool.acquire().await.unwrap();
        lease.batch("DROP TABLE aor_jobs").await.unwrap();
    }
    assert!(
        auth.register("rollback@example.test", "strong test password".into())
            .await
            .is_err()
    );
    {
        let mut lease = pool.acquire().await.unwrap();
        assert_eq!(Users::query(&mut lease).await.unwrap()[0].total, 0);
        assert_eq!(Counts::query(&mut lease).await.unwrap()[0].total, 0);
        lease.batch(aor_jobs::MIGRATION).await.unwrap();
    }
    auth.register("rollback@example.test", "strong test password".into())
        .await
        .unwrap();
    let provider = Provider::default();
    worker(&pool, provider.clone(), &clock)
        .work_once()
        .await
        .unwrap();
    auth.verify_email(&code(&provider, 0)).await.unwrap();
    pool.close();
    std::fs::remove_file(file).unwrap();
}
#[tokio::test]
async fn ambiguous_old_delivery_is_not_sent_after_provider_window() {
    let file = std::env::temp_dir().join(format!("aor-mail-window-{}.sqlite", Uuid::new_v4()));
    let pool = Pool::sqlite(&file, PoolOptions::default()).unwrap();
    aor_registry::migrate(&pool).await.unwrap();
    let clock = Arc::new(AtomicI64::new(1000));
    let auth = auth(&pool, &clock).await;
    auth.register("window@example.test", "strong test password".into())
        .await
        .unwrap();
    let provider = Provider::default();
    provider.lose_response.store(true, Ordering::SeqCst);
    let worker = worker(&pool, provider.clone(), &clock);
    assert!(worker.work_once().await.is_err());
    clock.store(1000 + aor_mail::REPLAY_WINDOW + 1, Ordering::SeqCst);
    worker.work_once().await.unwrap();
    assert_eq!(provider.attempts.lock().unwrap().len(), 1);
    let job = aor_jobs::list(&pool, "mail", 10)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(job.state, "failed");
    aor_jobs::requeue(&pool, &job.id, clock.load(Ordering::SeqCst))
        .await
        .unwrap();
    worker.work_once().await.unwrap();
    assert_eq!(provider.attempts.lock().unwrap().len(), 1);
    assert_eq!(
        aor_jobs::inspect(&pool, &job.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        "failed"
    );
    pool.close();
    std::fs::remove_file(file).unwrap();
}
#[tokio::test]
async fn wrong_key_and_expired_tokens_never_reach_transport() {
    let file = std::env::temp_dir().join(format!("aor-mail-expiry-{}.sqlite", Uuid::new_v4()));
    let pool = Pool::sqlite(&file, PoolOptions::default()).unwrap();
    aor_registry::migrate(&pool).await.unwrap();
    let clock = Arc::new(AtomicI64::new(1000));
    let auth = auth(&pool, &clock).await;
    auth.register("expiry@example.test", "strong test password".into())
        .await
        .unwrap();
    let provider = Provider::default();
    let c = clock.clone();
    let wrong = Worker::new(pool.clone(), Key::new([43; 32]).unwrap(), provider.clone())
        .with_clock(move || c.load(Ordering::SeqCst));
    assert!(wrong.work_once().await.is_err());
    assert!(provider.attempts.lock().unwrap().is_empty());
    let job = aor_jobs::list(&pool, "mail", 10)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(job.state, "failed");
    aor_jobs::requeue(&pool, &job.id, 1000).await.unwrap();
    clock.store(87400, Ordering::SeqCst);
    worker(&pool, provider.clone(), &clock)
        .work_once()
        .await
        .unwrap();
    assert!(provider.attempts.lock().unwrap().is_empty());
    let mut lease = pool.acquire().await.unwrap();
    assert_eq!(States::query(&mut lease).await.unwrap()[0].state, "expired");
    drop(lease);
    pool.close();
    std::fs::remove_file(file).unwrap();
}
#[tokio::test]
async fn production_requires_queue_and_keeps_public_responses_generic() {
    let file = std::env::temp_dir().join(format!("aor-mail-http-{}.sqlite", Uuid::new_v4()));
    let pool = Pool::sqlite(&file, PoolOptions::default()).unwrap();
    aor_registry::migrate(&pool).await.unwrap();
    assert!(
        aor_registry::router(pool.clone(), config(), None)
            .await
            .is_err()
    );
    let router = aor_registry::router_with_outbox(pool.clone(), config(), None, Some(outbox()))
        .await
        .unwrap();
    async fn request(
        router: &aor_router::Router,
        path: &str,
        input: serde_json::Value,
    ) -> (u16, Vec<u8>) {
        let result = router
            .handle(aor_http::Request {
                method: "POST".into(),
                target: path.into(),
                headers: vec![
                    ("Origin".into(), b"https://registry.example.test".to_vec()),
                    ("Content-Type".into(), b"application/json".to_vec()),
                ],
                body: aor_http::Body::from_bytes(input.to_string().into_bytes(), 4096).unwrap(),
            })
            .await;
        (
            result.status,
            match result.body {
                aor_http::ResponseBody::Bytes(bytes) => bytes,
                _ => panic!("unexpected stream"),
            },
        )
    }
    let signup = serde_json::json!({"email":"http@example.test","password":"a strong password"});
    let first = request(&router, "/accounts/register", signup.clone()).await;
    let again = request(&router, "/accounts/register", signup).await;
    assert_eq!(first.0, 202);
    assert_eq!(first, again);
    let known = request(
        &router,
        "/accounts/request-reset",
        serde_json::json!({"email":"http@example.test"}),
    )
    .await;
    let unknown = request(
        &router,
        "/accounts/request-reset",
        serde_json::json!({"email":"missing@example.test"}),
    )
    .await;
    assert_eq!(known.0, 202);
    assert_eq!(known, unknown);
    pool.close();
    std::fs::remove_file(file).unwrap();
}
