use std::os::unix::fs::PermissionsExt;
use std::{
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_aor-registry")
}
fn command(root: &Path) -> Command {
    let mut cmd = Command::new(binary());
    cmd.current_dir(root).env_clear();
    cmd
}
#[test]
fn key_help_is_read_only_and_generation_never_overwrites() {
    let root = std::env::temp_dir().join(format!("aor-key-cli-{}", aor_db::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let help = command(&root)
        .args(["mail-keygen", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(
        String::from_utf8(help.stdout)
            .unwrap()
            .starts_with("usage:")
    );
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    let generated = command(&root)
        .args(["mail-keygen", "signing.key"])
        .output()
        .unwrap();
    assert!(generated.status.success());
    let file = root.join("signing.key");
    let original = std::fs::read(&file).unwrap();
    assert_eq!(original.len(), 32);
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(
        !command(&root)
            .args(["mail-keygen", "signing.key"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(std::fs::read(&file).unwrap() == original);
    std::fs::remove_dir_all(root).unwrap();
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Unsupported {
    id: aor_db::Uuid,
}
impl aor_jobs::Job for Unsupported {
    const NAME: &'static str = "test.unsupported";
    const VERSION: i64 = 1;
    const IDEMPOTENCY: aor_jobs::Idempotent = aor_jobs::Idempotent::Natural;
    fn key(&self) -> String {
        self.id.to_string()
    }
}
#[test]
fn idle_worker_exits_cleanly_on_sigterm() {
    let root = std::env::temp_dir().join(format!("aor-worker-cli-{}", aor_db::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let database = root.join("registry.sqlite");
    let migrate = command(&root)
        .env("AOR_SQLITE", &database)
        .arg("migrate")
        .output()
        .unwrap();
    assert!(migrate.status.success());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let pool = aor_db::Pool::sqlite(&database, aor_db::PoolOptions::default()).unwrap();
    let job = runtime.block_on(async {
        let mut lease = pool.acquire().await.unwrap();
        let mut tx = lease.begin().await.unwrap();
        let id = aor_jobs::enqueue_in(
            &mut tx,
            &Unsupported {
                id: aor_db::Uuid::new_v4(),
            },
            "registry",
            aor_db::Utc::now().timestamp(),
            aor_jobs::Options::default(),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        id
    });
    let mut child = command(&root)
        .env("AOR_SQLITE", &database)
        .arg("work")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut ready = false;
    while Instant::now() < deadline {
        ready = runtime
            .block_on(aor_jobs::inspect(&pool, &job))
            .unwrap()
            .is_some_and(|row| row.state == "incompatible");
        if ready || child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if !ready {
        let _ = child.kill();
        let output = child.wait_with_output().unwrap();
        pool.close();
        std::fs::remove_dir_all(root).unwrap();
        panic!(
            "worker did not process the fixture: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(
        Command::new("/bin/kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut status = None;
    while Instant::now() < deadline {
        if let Some(done) = child.try_wait().unwrap() {
            status = Some(done);
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let final_status = child.wait().unwrap();
    assert!(status.is_some_and(|s| s.success()) && final_status.success());
    pool.close();
    std::fs::remove_dir_all(root).unwrap();
}
