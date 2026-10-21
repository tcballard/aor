use aor_sql::{Dialect, Schema, Type, check};
#[test]
fn scalar_select_checks_its_own_scope_and_types() {
    let mut s = Schema::default();
    s.apply("CREATE TABLE jobs (id TEXT NOT NULL PRIMARY KEY, state TEXT NOT NULL, attempt BIGINT NOT NULL);",Dialect::Postgres).unwrap();
    let q=check(&s,"UPDATE jobs SET attempt = attempt + 1 WHERE id = (SELECT id FROM jobs WHERE state = $1 ORDER BY id LIMIT 1 FOR UPDATE SKIP LOCKED) RETURNING id, attempt",Dialect::Postgres).unwrap();
    assert_eq!(q.parameters[0].ty, Type::Text);
    assert_eq!(q.columns[1].ty, Type::I64);
    for sql in [
        "UPDATE jobs SET attempt = 1 WHERE id = (SELECT id, state FROM jobs)",
        "UPDATE jobs SET attempt = 1 WHERE id = (SELECT missing FROM jobs)",
        "UPDATE jobs SET attempt = 1 WHERE id = (SELECT attempt FROM jobs)",
        "UPDATE jobs AS outer_job SET attempt = 1 WHERE id = (SELECT id FROM jobs WHERE state = outer_job.state)",
        "UPDATE jobs SET attempt = 1 WHERE id = (DELETE FROM jobs RETURNING id)",
    ] {
        assert!(check(&s, sql, Dialect::Postgres).is_err(), "{sql}");
    }
    assert!(check(&s,"UPDATE jobs SET attempt = 1 WHERE id = (SELECT id FROM jobs LIMIT 1 FOR UPDATE SKIP LOCKED)",Dialect::Sqlite).is_err());
}
