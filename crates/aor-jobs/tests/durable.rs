use aor_db::{Pool, PoolOptions, Uuid};
use aor_jobs::{self as jobs, Error, Idempotent, Job, Options};
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
struct Example {
    key: String,
}
impl Job for Example {
    const NAME: &'static str = "example";
    const VERSION: i64 = 1;
    const IDEMPOTENCY: Idempotent = Idempotent::ByKey;
    fn key(&self) -> String {
        self.key.clone()
    }
}
async fn enqueue(pool: &Pool, key: &str, attempts: i64) -> String {
    let mut lease = pool.acquire().await.unwrap();
    let mut tx = lease.begin().await.unwrap();
    let id = jobs::enqueue_in(
        &mut tx,
        &Example { key: key.into() },
        "test",
        100,
        Options {
            max_attempts: attempts,
            run_at: 100,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    id
}
async fn suite(pool: Pool) {
    jobs::migrate(&pool).await.unwrap();
    {
        let mut lease = pool.acquire().await.unwrap();
        let mut tx = lease.begin().await.unwrap();
        jobs::enqueue_in(
            &mut tx,
            &Example {
                key: "rollback".into(),
            },
            "test",
            100,
            Options::default(),
        )
        .await
        .unwrap();
        tx.rollback().await.unwrap();
    }
    assert!(jobs::claim(&pool, "test", 100, 10).await.unwrap().is_none());
    let id = enqueue(&pool, "fence", 3).await;
    assert_eq!(enqueue(&pool, "fence", 3).await, id);
    let old = jobs::claim(&pool, "test", 100, 10).await.unwrap().unwrap();
    assert_eq!(old.decode::<Example>().unwrap().key, "fence");
    assert!(jobs::claim(&pool, "test", 109, 10).await.unwrap().is_none());
    let new = jobs::claim(&pool, "test", 110, 10).await.unwrap().unwrap();
    assert_eq!(new.id(), id);
    assert_eq!(new.attempt(), 2);
    assert!(matches!(
        jobs::renew(&pool, &old, 111, 10).await,
        Err(Error::Stale)
    ));
    assert!(matches!(
        jobs::fail(&pool, &old, 111, jobs::Failure::Handler).await,
        Err(Error::Stale)
    ));
    {
        let mut lease = pool.acquire().await.unwrap();
        let mut tx = lease.begin().await.unwrap();
        assert!(matches!(
            jobs::fence_in(&mut tx, &old, 111).await,
            Err(Error::Stale)
        ));
        assert!(matches!(
            jobs::complete_in(&mut tx, &old, 111).await,
            Err(Error::Stale)
        ));
        tx.rollback().await.unwrap();
    }
    jobs::renew(&pool, &new, 115, 20).await.unwrap();
    assert!(jobs::claim(&pool, "test", 120, 10).await.unwrap().is_none());
    {
        let mut lease = pool.acquire().await.unwrap();
        let mut tx = lease.begin().await.unwrap();
        jobs::fence_in(&mut tx, &new, 121).await.unwrap();
        jobs::complete_in(&mut tx, &new, 121).await.unwrap();
        tx.rollback().await.unwrap();
    }
    assert_eq!(
        jobs::inspect(&pool, &id).await.unwrap().unwrap().state,
        "running"
    );
    {
        let mut lease = pool.acquire().await.unwrap();
        let mut tx = lease.begin().await.unwrap();
        jobs::fence_in(&mut tx, &new, 122).await.unwrap();
        jobs::complete_in(&mut tx, &new, 122).await.unwrap();
        tx.commit().await.unwrap();
    }
    assert_eq!(
        jobs::inspect(&pool, &id).await.unwrap().unwrap().state,
        "succeeded"
    );
    let failed = enqueue(&pool, "failure", 2).await;
    let claim = jobs::claim(&pool, "test", 200, 10).await.unwrap().unwrap();
    jobs::fail(&pool, &claim, 201, jobs::Failure::Dependency)
        .await
        .unwrap();
    let row = jobs::inspect(&pool, &failed).await.unwrap().unwrap();
    assert_eq!(row.state, "queued");
    assert!((203..=205).contains(&row.run_at));
    assert!(jobs::claim(&pool, "test", 202, 10).await.unwrap().is_none());
    let claim = jobs::claim(&pool, "test", 210, 10).await.unwrap().unwrap();
    jobs::fail(&pool, &claim, 211, jobs::Failure::Handler)
        .await
        .unwrap();
    assert_eq!(
        jobs::inspect(&pool, &failed).await.unwrap().unwrap().state,
        "failed"
    );
    jobs::requeue(&pool, &failed, 220).await.unwrap();
    let claim = jobs::claim(&pool, "test", 220, 10).await.unwrap().unwrap();
    jobs::incompatible(&pool, &claim, 221).await.unwrap();
    let row = jobs::inspect(&pool, &failed).await.unwrap().unwrap();
    assert_eq!(row.state, "incompatible");
    assert_eq!(row.attempt, 0);
    assert!(jobs::claim(&pool, "test", 230, 10).await.unwrap().is_none());
    jobs::requeue(&pool, &failed, 230).await.unwrap();
    let claim = jobs::claim(&pool, "test", 230, 10).await.unwrap().unwrap();
    jobs::park(&pool, &failed, 231).await.unwrap();
    assert!(matches!(
        jobs::renew(&pool, &claim, 231, 10).await,
        Err(Error::Stale)
    ));
    let dead = enqueue(&pool, "dead", 1).await;
    jobs::claim(&pool, "test", 300, 10).await.unwrap().unwrap();
    assert!(jobs::claim(&pool, "test", 310, 10).await.unwrap().is_none());
    assert_eq!(
        jobs::inspect(&pool, &dead).await.unwrap().unwrap().state,
        "failed"
    );
    let concurrent = enqueue(&pool, "concurrent", 3).await;
    let (a, b) = tokio::join!(
        jobs::claim(&pool, "test", 400, 10),
        jobs::claim(&pool, "test", 400, 10)
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(usize::from(a.is_some()) + usize::from(b.is_some()), 1);
    assert_eq!(a.or(b).unwrap().id(), concurrent);
    pool.close();
}
#[tokio::test]
async fn sqlite_durable_queue() {
    let file = std::env::temp_dir().join(format!("aor-jobs-{}.sqlite", Uuid::new_v4()));
    suite(Pool::sqlite(&file, PoolOptions::default()).unwrap()).await;
    std::fs::remove_file(file).unwrap();
}
#[tokio::test]
#[ignore = "requires isolated AOR_TEST_DATABASE_URL"]
async fn postgres_durable_queue() {
    suite(
        Pool::postgres(
            &std::env::var("AOR_TEST_DATABASE_URL").unwrap(),
            PoolOptions::default(),
        )
        .unwrap(),
    )
    .await;
}
