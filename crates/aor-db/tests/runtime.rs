use aor_db::*;
use std::time::Duration;
sql!(pub InsertLocal, sqlite, "tests/migrations/sqlite", "INSERT INTO items (id, title, note) VALUES ($1, $2, $3) RETURNING id, version");
sql!(pub ListLocal, sqlite, "tests/migrations/sqlite", "SELECT id, title, note, version FROM items ORDER BY title");
sql!(pub InsertPg, postgres, "tests/migrations/postgres", "INSERT INTO items (id, title, note) VALUES ($1, $2, $3) RETURNING id, version");
sql!(pub ListPg, postgres, "tests/migrations/postgres", "SELECT id, title, note, version FROM items ORDER BY title");
sql!(pub UpdatePg, postgres, "tests/migrations/postgres", "UPDATE items SET title = $1, version = version + 1 WHERE id = $2 AND version = $3 RETURNING version");
fn options() -> PoolOptions {
    PoolOptions {
        max_connections: 1,
        acquire_timeout: Duration::from_millis(100),
        query_timeout: Duration::from_secs(2),
        ..Default::default()
    }
}
#[tokio::test]
async fn sqlite_transactions_persistence_bounds_and_counts() {
    let file = std::env::temp_dir().join(format!("aor-db-{}.sqlite", Uuid::new_v4()));
    let pool = Pool::sqlite(&file, options()).unwrap();
    let migrations = aor_migrate::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/migrations/sqlite"),
        Dialect::Sqlite,
    )
    .unwrap();
    assert_eq!(aor_migrate::apply(&pool, &migrations).await.unwrap(), 1);
    assert_eq!(aor_migrate::apply(&pool, &migrations).await.unwrap(), 0);
    let mut lease = pool.acquire().await.unwrap();
    assert!(matches!(pool.acquire().await, Err(Error::Timeout)));
    let counter = QueryCounter::default();
    counter
        .scope(async {
            let mut tx = lease.begin().await.unwrap();
            InsertLocal::query(&mut tx, "one".into(), "First".into(), None)
                .await
                .unwrap();
            tx.commit().await.unwrap();
            for _ in 0..3 {
                assert_eq!(ListLocal::query(&mut lease).await.unwrap().len(), 1);
            }
        })
        .await;
    assert_eq!(counter.total(), 4);
    assert_eq!(counter.repeated(3).len(), 1);
    {
        let mut tx = lease.begin().await.unwrap();
        InsertLocal::execute(
            &mut tx,
            "two".into(),
            "Rolled back".into(),
            Some("note".into()),
        )
        .await
        .unwrap();
        tx.rollback().await.unwrap();
    }
    {
        let mut tx = lease.begin().await.unwrap();
        InsertLocal::execute(&mut tx, "three".into(), "Dropped".into(), None)
            .await
            .unwrap();
    }
    assert!(matches!(
        ListLocal::query(&mut lease).await,
        Err(Error::Closed)
    ));
    drop(lease);
    let mut lease = pool.acquire().await.unwrap();
    let rows = ListLocal::query(&mut lease).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].version, 1);
    assert_eq!(rows[0].note, None);
    assert!(matches!(
        ListPg::query(&mut lease).await,
        Err(Error::Dialect)
    ));
    drop(lease);
    pool.close();
    assert!(matches!(pool.acquire().await, Err(Error::Closed)));
    let pool = Pool::sqlite(&file, options()).unwrap();
    assert_eq!(
        ListLocal::query(&mut pool.acquire().await.unwrap())
            .await
            .unwrap()
            .len(),
        1
    );
    pool.close();
    std::fs::remove_file(file).unwrap();
}
#[tokio::test]
async fn migration_tampering_and_atomic_failure() {
    let file = std::env::temp_dir().join(format!("aor-migrate-{}.sqlite", Uuid::new_v4()));
    let pool = Pool::sqlite(&file, options()).unwrap();
    let mut migrations = aor_migrate::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/migrations/sqlite"),
        Dialect::Sqlite,
    )
    .unwrap();
    aor_migrate::apply(&pool, &migrations).await.unwrap();
    migrations[0].checksum.push('x');
    assert!(
        aor_migrate::apply(&pool, &migrations)
            .await
            .unwrap_err()
            .to_string()
            .contains("checksum")
    );
    migrations[0].checksum.pop();
    let mut bad = migrations[0].clone();
    bad.name = "002_bad.sql".into();
    bad.sql = "CREATE TABLE transient (id TEXT); INSERT INTO missing VALUES (1);".into();
    migrations.push(bad);
    assert!(aor_migrate::apply(&pool, &migrations).await.is_err());
    let mut lease = pool.acquire().await.unwrap();
    assert!(
        lease
            .query(Dialect::Sqlite, "SELECT id FROM transient", &[])
            .await
            .is_err()
    );
    drop(lease);
    pool.close();
    std::fs::remove_file(file).unwrap();
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL; mandatory in database CI job"]
async fn postgres_transactions_migration_locking_and_checked_queries() {
    let url = std::env::var("AOR_TEST_DATABASE_URL").expect("isolated test database URL required");
    let pool = Pool::postgres(&url, options()).unwrap();
    pool.acquire()
        .await
        .unwrap()
        .batch("DROP TABLE IF EXISTS items; DROP TABLE IF EXISTS _aor_migrations;")
        .await
        .unwrap();
    let migrations = aor_migrate::load(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/migrations/postgres"),
        Dialect::Postgres,
    )
    .unwrap();
    let other = Pool::postgres(&url, options()).unwrap();
    let (a, b) = tokio::join!(
        aor_migrate::apply(&pool, &migrations),
        aor_migrate::apply(&other, &migrations)
    );
    assert_eq!(a.unwrap() + b.unwrap(), 1);
    let mut lease = pool.acquire().await.unwrap();
    let id = Uuid::new_v4();
    let mut tx = lease.begin().await.unwrap();
    let inserted = InsertPg::query(&mut tx, id, "Persisted edition".into(), None)
        .await
        .unwrap();
    assert_eq!(inserted[0].id, id);
    tx.commit().await.unwrap();
    assert_eq!(
        UpdatePg::query(&mut lease, "Revised".into(), id, 1)
            .await
            .unwrap()[0]
            .version,
        2
    );
    assert!(
        UpdatePg::query(&mut lease, "Stale".into(), id, 1)
            .await
            .unwrap()
            .is_empty()
    );
    {
        let mut tx = lease.begin().await.unwrap();
        InsertPg::execute(&mut tx, Uuid::new_v4(), "Discarded".into(), None)
            .await
            .unwrap();
    }
    drop(lease);
    assert_eq!(
        ListPg::query(&mut pool.acquire().await.unwrap())
            .await
            .unwrap()
            .len(),
        1
    );
    // Cancelled queries discard their connection and release the pool permit.
    let mut lease = pool.acquire().await.unwrap();
    let timed = tokio::time::timeout(
        Duration::from_millis(20),
        lease.query(Dialect::Postgres, "SELECT pg_sleep(1)", &[]),
    )
    .await;
    assert!(timed.is_err());
    drop(lease);
    assert_eq!(
        ListPg::query(&mut pool.acquire().await.unwrap())
            .await
            .unwrap()
            .len(),
        1
    );
    pool.close();
    other.close();
}
