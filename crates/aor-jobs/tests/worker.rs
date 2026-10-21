use aor_db::{Pool, PoolOptions, Uuid};
use aor_jobs::{
    self as jobs, Context, Error, Handler, Idempotent, Job, Options, Worker, WorkerOptions,
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering},
    },
    time::Duration,
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Example<const VERSION: i64> {
    key: String,
}
impl<const VERSION: i64> Job for Example<VERSION> {
    const NAME: &'static str = "worker.example";
    const VERSION: i64 = VERSION;
    const IDEMPOTENCY: Idempotent = Idempotent::ByKey;
    fn key(&self) -> String {
        self.key.clone()
    }
}
#[derive(Serialize, Deserialize)]
struct Malformed {
    unexpected: String,
}
impl Job for Malformed {
    const NAME: &'static str = Example::<1>::NAME;
    const VERSION: i64 = 1;
    const IDEMPOTENCY: Idempotent = Idempotent::Natural;
    fn key(&self) -> String {
        self.unexpected.clone()
    }
}
aor_db::sql!(
    InsertEffect,
    portable,
    "tests/worker-migrations",
    "INSERT INTO worker_effects (id) VALUES ($1) ON CONFLICT (id) DO NOTHING"
);
aor_db::sql!(
    CountEffects,
    portable,
    "tests/worker-migrations",
    "SELECT COUNT(id) AS total FROM worker_effects WHERE id = $1"
);
#[derive(Clone, Default)]
struct Gate {
    started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    dropped: Arc<AtomicBool>,
}
struct DropFlag(Arc<AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
#[derive(Clone, Default)]
struct Effect {
    calls: Arc<AtomicUsize>,
    gate: Option<Gate>,
    locked: bool,
    fail: bool,
}
impl<const VERSION: i64> Handler<Example<VERSION>> for Effect {
    async fn handle(&self, context: Context<'_>, job: Example<VERSION>) -> jobs::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let _guard = self.gate.as_ref().map(|g| DropFlag(g.dropped.clone()));
        if let Some(gate) = &self.gate {
            if !self.locked {
                gate.started.notify_one();
                gate.release.notified().await;
            }
        }
        let mut lease = context.pool.acquire().await?;
        let mut tx = lease.begin().await?;
        jobs::fence_in(&mut tx, context.claim, context.now()).await?;
        InsertEffect::execute(&mut tx, job.key).await?;
        if let Some(gate) = &self.gate {
            if self.locked {
                gate.started.notify_one();
                gate.release.notified().await;
            }
        }
        if self.fail {
            return Err(Error::Invalid);
        }
        jobs::complete_in(&mut tx, context.claim, context.now()).await?;
        tx.commit().await?;
        Ok(())
    }
}
fn options() -> WorkerOptions {
    WorkerOptions {
        heartbeat: Duration::from_millis(10),
        poll: Duration::from_millis(2),
        drain: Duration::from_millis(100),
        ..WorkerOptions::default()
    }
}
fn worker(pool: &Pool, clock: &Arc<AtomicI64>, effect: Effect) -> Worker {
    let time = clock.clone();
    let mut worker = Worker::new(pool.clone())
        .with_options(options())
        .unwrap()
        .with_clock(move || time.load(Ordering::SeqCst));
    worker.register::<Example<1>>("one", effect).unwrap();
    worker
}
async fn enqueue<J: Job>(pool: &Pool, queue: &str, job: &J, at: i64) -> String {
    let mut lease = pool.acquire().await.unwrap();
    let mut tx = lease.begin().await.unwrap();
    let id = jobs::enqueue_in(
        &mut tx,
        job,
        queue,
        at,
        Options {
            max_attempts: 2,
            run_at: at,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    id
}
async fn example(pool: &Pool, queue: &str, key: &str, at: i64) -> String {
    enqueue(pool, queue, &Example::<1> { key: key.into() }, at).await
}
async fn count(pool: &Pool, key: &str) -> i64 {
    let mut lease = pool.acquire().await.unwrap();
    CountEffects::query(&mut lease, key.into()).await.unwrap()[0].total
}
async fn state(pool: &Pool, id: &str) -> String {
    jobs::inspect(pool, id).await.unwrap().unwrap().state
}
async fn suite(pool: Pool, source: &str) {
    let migrations = aor_migrate::embedded(
        &[
            ("001_jobs.sql", jobs::MIGRATION),
            (
                "002_effects.sql",
                include_str!("worker-migrations/001_effects.sql"),
            ),
        ],
        pool.dialect(),
    )
    .unwrap();
    aor_migrate::apply(&pool, &migrations).await.unwrap();
    let clock = Arc::new(AtomicI64::new(1000));
    let effect = Effect::default();
    let mut runtime = worker(&pool, &clock, effect.clone());
    assert!(
        runtime
            .register::<Example<1>>("one", Effect::default())
            .is_err()
    );
    assert!(
        runtime
            .register::<Example<1>>("bad queue", Effect::default())
            .is_err()
    );
    runtime
        .register::<Example<2>>("one", effect.clone())
        .unwrap();
    runtime
        .register::<Example<1>>("two", effect.clone())
        .unwrap();
    let a = example(&pool, "one", "fair-a", 1000).await;
    let b = example(&pool, "one", "fair-b", 1000).await;
    let c = example(&pool, "two", "fair-c", 1000).await;
    let first = runtime.claim().await.unwrap().unwrap();
    assert_eq!(first.queue(), "one");
    runtime.process(&first).await.unwrap();
    let second = runtime.claim().await.unwrap().unwrap();
    assert_eq!(second.id(), c);
    runtime.process(&second).await.unwrap();
    assert!(runtime.work_once().await.unwrap());
    for (id, key) in [(&a, "fair-a"), (&b, "fair-b"), (&c, "fair-c")] {
        assert_eq!(state(&pool, id).await, "succeeded");
        assert_eq!(count(&pool, key).await, 1);
    }
    assert!(matches!(runtime.process(&first).await, Err(Error::Stale)));
    assert_eq!(effect.calls.load(Ordering::SeqCst), 4);
    assert_eq!(count(&pool, "fair-a").await, 1);
    let v2 = enqueue(
        &pool,
        "one",
        &Example::<2> {
            key: "version-two".into(),
        },
        1000,
    )
    .await;
    assert!(runtime.work_once().await.unwrap());
    assert_eq!(state(&pool, &v2).await, "succeeded");
    let calls = effect.calls.load(Ordering::SeqCst);
    let future = enqueue(
        &pool,
        "one",
        &Example::<3> {
            key: "future".into(),
        },
        1000,
    )
    .await;
    assert!(runtime.work_once().await.unwrap());
    let row = jobs::inspect(&pool, &future).await.unwrap().unwrap();
    assert_eq!(row.state, "incompatible");
    assert_eq!(row.attempt, 0);
    let other_queue = example(&pool, "unregistered", "queue-isolation", 1000).await;
    let claim = jobs::claim(&pool, "unregistered", 1000, 60)
        .await
        .unwrap()
        .unwrap();
    runtime.process(&claim).await.unwrap();
    assert_eq!(state(&pool, &other_queue).await, "incompatible");
    assert_eq!(effect.calls.load(Ordering::SeqCst), calls);
    let malformed = enqueue(
        &pool,
        "one",
        &Malformed {
            unexpected: "malformed".into(),
        },
        1000,
    )
    .await;
    assert!(matches!(runtime.work_once().await, Err(Error::Invalid)));
    assert_eq!(effect.calls.load(Ordering::SeqCst), calls);
    assert_eq!(
        jobs::inspect(&pool, &malformed)
            .await
            .unwrap()
            .unwrap()
            .last_error,
        "AOR-JOB-PAYLOAD"
    );
    jobs::park(&pool, &malformed, 1000).await.unwrap();

    // Failure after a domain write must drop the transaction before retrying.
    let id = example(&pool, "one", "rollback", 1000).await;
    let mut failing = worker(
        &pool,
        &clock,
        Effect {
            fail: true,
            ..Effect::default()
        },
    );
    assert!(matches!(failing.work_once().await, Err(Error::Invalid)));
    assert_eq!(count(&pool, "rollback").await, 0);
    assert_eq!(state(&pool, &id).await, "queued");
    clock.store(1010, Ordering::SeqCst);
    assert!(matches!(failing.work_once().await, Err(Error::Invalid)));
    assert_eq!(state(&pool, &id).await, "failed");
    assert_eq!(count(&pool, "rollback").await, 0);

    // A slow external phase stays owned past its original lease.
    clock.store(2000, Ordering::SeqCst);
    let id = example(&pool, "one", "heartbeat", 2000).await;
    let gate = Gate::default();
    let mut runtime = worker(
        &pool,
        &clock,
        Effect {
            gate: Some(gate.clone()),
            ..Effect::default()
        },
    );
    let claim = runtime.claim().await.unwrap().unwrap();
    let monitor = async {
        gate.started.notified().await;
        clock.store(2030, Ordering::SeqCst);
        loop {
            if jobs::inspect(&pool, &id)
                .await
                .unwrap()
                .unwrap()
                .lease_expires_at
                == 2090
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert!(jobs::claim(&pool, "one", 2060, 60).await.unwrap().is_none());
        clock.store(2061, Ordering::SeqCst);
        gate.release.notify_one();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(runtime.process(&claim), monitor)
    })
    .await
    .unwrap();
    result.unwrap();
    assert_eq!(state(&pool, &id).await, "succeeded");
    assert_eq!(count(&pool, "heartbeat").await, 1);

    // An expired heartbeat drops the handler before it can publish an effect.
    clock.store(3000, Ordering::SeqCst);
    let id = example(&pool, "one", "lost", 3000).await;
    let gate = Gate::default();
    let mut runtime = worker(
        &pool,
        &clock,
        Effect {
            gate: Some(gate.clone()),
            ..Effect::default()
        },
    );
    let claim = runtime.claim().await.unwrap().unwrap();
    let monitor = async {
        gate.started.notified().await;
        clock.store(3060, Ordering::SeqCst);
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(runtime.process(&claim), monitor)
    })
    .await
    .unwrap();
    assert!(matches!(result, Err(Error::Stale)));
    assert!(gate.dropped.load(Ordering::SeqCst));
    assert_eq!(state(&pool, &id).await, "running");
    assert_eq!(count(&pool, "lost").await, 0);
    let mut replacement = worker(&pool, &clock, Effect::default());
    replacement.work_once().await.unwrap();
    assert_eq!(count(&pool, "lost").await, 1);
    assert!(matches!(
        replacement.process(&claim).await,
        Err(Error::Stale)
    ));

    // A heartbeat waiting on the handler's own transaction must not deadlock it.
    clock.store(4000, Ordering::SeqCst);
    let id = example(&pool, "one", "held-lock", 4000).await;
    let gate = Gate::default();
    let mut runtime = worker(
        &pool,
        &clock,
        Effect {
            gate: Some(gate.clone()),
            locked: true,
            ..Effect::default()
        },
    );
    let claim = runtime.claim().await.unwrap().unwrap();
    let release = async {
        gate.started.notified().await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        gate.release.notify_one();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(runtime.process(&claim), release)
    })
    .await
    .unwrap();
    result.unwrap();
    assert_eq!(state(&pool, &id).await, "succeeded");
    assert_eq!(count(&pool, "held-lock").await, 1);

    // Shutdown drains one active job and never starts the queued successor.
    clock.store(5000, Ordering::SeqCst);
    let id = example(&pool, "one", "drained", 5000).await;
    let pending = example(&pool, "one", "not-started", 5001).await;
    let gate = Gate::default();
    let mut runtime = worker(
        &pool,
        &clock,
        Effect {
            gate: Some(gate.clone()),
            ..Effect::default()
        },
    );
    let stop = async {
        gate.started.notified().await;
    };
    let release = async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        clock.store(5001, Ordering::SeqCst);
        gate.release.notify_one();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(runtime.run_until(stop), release)
    })
    .await
    .unwrap();
    result.unwrap();
    assert_eq!(state(&pool, &id).await, "succeeded");
    assert_eq!(state(&pool, &pending).await, "queued");
    runtime.run_until(std::future::ready(())).await.unwrap();
    assert_eq!(state(&pool, &pending).await, "queued");
    jobs::park(&pool, &pending, 5000).await.unwrap();

    // Drain timeout rolls back an unfinished transaction; recovery makes one effect.
    clock.store(6000, Ordering::SeqCst);
    let id = example(&pool, "one", "timed-out", 6000).await;
    let gate = Gate::default();
    let mut runtime = worker(
        &pool,
        &clock,
        Effect {
            gate: Some(gate.clone()),
            locked: true,
            ..Effect::default()
        },
    )
    .with_options(WorkerOptions {
        drain: Duration::from_millis(25),
        ..options()
    })
    .unwrap();
    tokio::time::timeout(
        Duration::from_secs(3),
        runtime.run_until(async {
            gate.started.notified().await;
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(gate.dropped.load(Ordering::SeqCst));
    assert_eq!(state(&pool, &id).await, "running");
    assert_eq!(count(&pool, "timed-out").await, 0);
    clock.store(6061, Ordering::SeqCst);
    let mut replacement = worker(&pool, &clock, Effect::default());
    assert!(replacement.work_once().await.unwrap());
    assert_eq!(state(&pool, &id).await, "succeeded");
    assert_eq!(count(&pool, "timed-out").await, 1);
    assert!(!replacement.work_once().await.unwrap());
    // Kill a real process after its fenced insert but before commit. The
    // replacement must observe rollback, reclaim and commit exactly one effect.
    clock.store(7000, Ordering::SeqCst);
    let id = example(&pool, "one", "killed-transaction", 7000).await;
    let ready = std::env::temp_dir().join(format!("aor-worker-ready-{}", Uuid::new_v4()));
    let mut child = Child(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "child_worker_holds_transaction", "--nocapture"])
            .env("AOR_WORKER_CHILD_SOURCE", source)
            .env(
                "AOR_WORKER_CHILD_POSTGRES",
                if pool.dialect() == aor_db::Dialect::Postgres {
                    "1"
                } else {
                    "0"
                },
            )
            .env("AOR_WORKER_CHILD_READY", &ready)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        while !ready.exists() {
            if let Some(status) = child.0.try_wait().unwrap() {
                panic!("worker exited before insert: {status}");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    child.0.kill().unwrap();
    assert!(!child.0.wait().unwrap().success());
    assert_eq!(state(&pool, &id).await, "running");
    assert_eq!(count(&pool, "killed-transaction").await, 0);
    clock.store(7061, Ordering::SeqCst);
    let mut replacement = worker(&pool, &clock, Effect::default());
    replacement.work_once().await.unwrap();
    assert!(!replacement.work_once().await.unwrap());
    assert_eq!(state(&pool, &id).await, "succeeded");
    assert_eq!(count(&pool, "killed-transaction").await, 1);
    std::fs::remove_file(ready).unwrap();
    pool.close();
}
struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[tokio::test]
async fn child_worker_holds_transaction() {
    let Ok(source) = std::env::var("AOR_WORKER_CHILD_SOURCE") else {
        return;
    };
    let pool = if std::env::var("AOR_WORKER_CHILD_POSTGRES").as_deref() == Ok("1") {
        Pool::postgres(&source, PoolOptions::default()).unwrap()
    } else {
        Pool::sqlite(&source, PoolOptions::default()).unwrap()
    };
    let clock = Arc::new(AtomicI64::new(7000));
    let gate = Gate::default();
    let mut runtime = worker(
        &pool,
        &clock,
        Effect {
            gate: Some(gate.clone()),
            locked: true,
            ..Effect::default()
        },
    );
    let ready = async {
        gate.started.notified().await;
        std::fs::write(
            std::env::var_os("AOR_WORKER_CHILD_READY").unwrap(),
            b"inserted but not committed",
        )
        .unwrap();
        std::future::pending::<()>().await;
    };
    let _ = tokio::join!(runtime.work_once(), ready);
}
#[tokio::test]
async fn sqlite_worker_fault_matrix() {
    let file = std::env::temp_dir().join(format!("aor-worker-{}.sqlite", Uuid::new_v4()));
    suite(
        Pool::sqlite(&file, PoolOptions::default()).unwrap(),
        file.to_str().unwrap(),
    )
    .await;
    std::fs::remove_file(file).unwrap();
}
#[tokio::test]
#[ignore = "requires isolated AOR_TEST_DATABASE_URL"]
async fn postgres_worker_fault_matrix() {
    suite(
        Pool::postgres(
            &std::env::var("AOR_TEST_DATABASE_URL").unwrap(),
            PoolOptions::default(),
        )
        .unwrap(),
        &std::env::var("AOR_TEST_DATABASE_URL").unwrap(),
    )
    .await;
}
#[tokio::test]
async fn invalid_options_and_empty_registration_fail_closed() {
    let file = std::env::temp_dir().join(format!("aor-worker-options-{}.sqlite", Uuid::new_v4()));
    let pool = Pool::sqlite(&file, PoolOptions::default()).unwrap();
    for options in [
        WorkerOptions {
            lease_seconds: 0,
            ..WorkerOptions::default()
        },
        WorkerOptions {
            heartbeat: Duration::ZERO,
            ..WorkerOptions::default()
        },
        WorkerOptions {
            heartbeat: Duration::from_secs(21),
            ..WorkerOptions::default()
        },
        WorkerOptions {
            poll: Duration::ZERO,
            ..WorkerOptions::default()
        },
        WorkerOptions {
            drain: Duration::from_secs(3601),
            ..WorkerOptions::default()
        },
    ] {
        assert!(Worker::new(pool.clone()).with_options(options).is_err());
    }
    let mut worker = Worker::new(pool.clone());
    assert!(matches!(worker.claim().await, Err(Error::Invalid)));
    assert!(matches!(
        worker.run_until(std::future::ready(())).await,
        Err(Error::Invalid)
    ));
    assert!(!file.exists());
    pool.close();
}

#[test]
fn queue_errors_redact_database_details_in_display_and_debug() {
    let error = Error::Database(aor_db::Error::Configuration(
        "secret payload or provider response".into(),
    ));
    assert_eq!(format!("{error}"), "AOR-JOB-DATABASE");
    assert_eq!(format!("{error:?}"), "AOR-JOB-DATABASE");
}
