use super::*;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
struct DropFlag(Arc<AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
#[derive(Clone)]
struct Slow {
    started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    dropped: Arc<AtomicBool>,
}
impl Transport for Slow {
    async fn send(&self, _message: &Message) -> Result<()> {
        let _guard = DropFlag(self.dropped.clone());
        self.started.notify_one();
        self.release.notified().await;
        Ok(())
    }
}
async fn fixture() -> (Pool, Claim, std::path::PathBuf) {
    let file = std::env::temp_dir().join(format!("aor-heartbeat-{}.sqlite", Uuid::new_v4()));
    let pool = Pool::sqlite(&file, aor_db::PoolOptions::default()).unwrap();
    let migrations = aor_migrate::embedded(
        &[
            ("001_jobs.sql", aor_jobs::MIGRATION),
            ("002_mail.sql", MIGRATION),
        ],
        pool.dialect(),
    )
    .unwrap();
    aor_migrate::apply(&pool, &migrations).await.unwrap();
    {
        let mut lease = pool.acquire().await.unwrap();
        let mut tx = lease.begin().await.unwrap();
        Outbox::new(Key::new([42; 32]).unwrap(), "from@example.test".into())
            .unwrap()
            .enqueue_in(&mut tx, "to@example.test", "verify", 87400, 1000)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    let claim = aor_jobs::claim(&pool, "mail", 1000, 60)
        .await
        .unwrap()
        .unwrap();
    (pool, claim, file)
}
#[tokio::test]
async fn heartbeat_extends_the_fence_during_external_work() {
    let (pool, claim, file) = fixture().await;
    let clock = Arc::new(AtomicI64::new(1000));
    let transport = Slow {
        started: Arc::new(tokio::sync::Notify::new()),
        release: Arc::new(tokio::sync::Notify::new()),
        dropped: Arc::new(AtomicBool::new(false)),
    };
    let c = clock.clone();
    let mut worker = Worker::new(pool.clone(), Key::new([42; 32]).unwrap(), transport.clone())
        .with_clock(move || c.load(Ordering::SeqCst));
    worker.heartbeat = Duration::from_millis(10);
    let monitor = async {
        transport.started.notified().await;
        clock.store(1030, Ordering::SeqCst);
        let mut renewed = false;
        for _ in 0..100 {
            if aor_jobs::inspect(&pool, claim.id())
                .await
                .unwrap()
                .unwrap()
                .lease_expires_at
                == 1090
            {
                renewed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(renewed);
        assert!(
            aor_jobs::claim(&pool, "mail", 1060, 60)
                .await
                .unwrap()
                .is_none()
        );
        clock.store(1061, Ordering::SeqCst);
        transport.release.notify_one();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(worker.run_claim(&claim), monitor)
    })
    .await
    .unwrap();
    result.unwrap();
    assert_eq!(
        aor_jobs::inspect(&pool, claim.id())
            .await
            .unwrap()
            .unwrap()
            .state,
        "succeeded"
    );
    pool.close();
    std::fs::remove_file(file).unwrap();
}
#[tokio::test]
async fn lost_lease_cancels_external_work_and_cannot_acknowledge() {
    let (pool, claim, file) = fixture().await;
    let clock = Arc::new(AtomicI64::new(1000));
    let transport = Slow {
        started: Arc::new(tokio::sync::Notify::new()),
        release: Arc::new(tokio::sync::Notify::new()),
        dropped: Arc::new(AtomicBool::new(false)),
    };
    let c = clock.clone();
    let mut worker = Worker::new(pool.clone(), Key::new([42; 32]).unwrap(), transport.clone())
        .with_clock(move || c.load(Ordering::SeqCst));
    worker.heartbeat = Duration::from_millis(10);
    let monitor = async {
        transport.started.notified().await;
        clock.store(1060, Ordering::SeqCst);
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(worker.run_claim(&claim), monitor)
    })
    .await
    .unwrap();
    assert!(matches!(result, Err(Error::Queue(aor_jobs::Error::Stale))));
    assert!(transport.dropped.load(Ordering::SeqCst));
    assert_eq!(
        aor_jobs::inspect(&pool, claim.id())
            .await
            .unwrap()
            .unwrap()
            .state,
        "running"
    );
    pool.close();
    std::fs::remove_file(file).unwrap();
}
#[test]
fn secret_files_are_private_regular_files_and_keys_never_overwrite() {
    let dir = std::env::temp_dir().join(format!("aor-mail-keys-{}", Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("key");
    generate_key(&path).unwrap();
    assert!(Key::load(&path).is_ok());
    assert!(generate_key(&path).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(Key::load(&path).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::os::unix::fs::symlink(&path, dir.join("link")).unwrap();
    assert!(Key::load(&dir.join("link")).is_err());
    assert!(Key::load(&dir).is_err());
    std::fs::write(&path, [0; 32]).unwrap();
    assert!(Key::load(&path).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}
