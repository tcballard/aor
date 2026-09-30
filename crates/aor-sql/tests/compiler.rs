use aor_sql::{Dialect::*, *};
fn schema() -> Schema {
    let mut s = Schema::default();
    s.apply("CREATE TABLE editions (id UUID PRIMARY KEY, slug TEXT NOT NULL UNIQUE, title TEXT NOT NULL, body TEXT, version BIGINT NOT NULL DEFAULT 1, published_at TIMESTAMPTZ NOT NULL); CREATE TABLE comments (id UUID PRIMARY KEY, edition_id UUID NOT NULL REFERENCES editions(id), score INTEGER NOT NULL);",Postgres).unwrap();
    s
}
#[test]
fn ddl_is_atomic_and_applies_ordered_changes() {
    let mut s = schema();
    s.apply("ALTER TABLE editions ADD COLUMN summary TEXT; ALTER TABLE editions RENAME COLUMN summary TO subtitle; ALTER TABLE editions ALTER COLUMN subtitle SET NOT NULL; CREATE INDEX edition_titles ON editions (title);",Postgres).unwrap();
    assert!(!s.tables["editions"].columns["subtitle"].nullable);
    let before = serde_json::to_string(&s).unwrap();
    assert!(
        s.apply(
            "ALTER TABLE editions ADD COLUMN extra TEXT; VACUUM;",
            Postgres
        )
        .is_err()
    );
    assert_eq!(before, serde_json::to_string(&s).unwrap());
    assert!(
        s.apply("ALTER TABLE editions DROP COLUMN title;", Postgres)
            .is_err()
    );
    s.apply(
        "DROP INDEX edition_titles; ALTER TABLE editions DROP COLUMN subtitle;",
        Postgres,
    )
    .unwrap();
}
#[test]
fn resolves_parameters_and_nullable_results() {
    let q=check(&schema(),"SELECT id, title, body FROM editions WHERE slug = $1 AND version > $2 ORDER BY published_at DESC LIMIT $3",Postgres).unwrap();
    assert_eq!(
        q.parameters
            .iter()
            .map(|p| p.ty.clone())
            .collect::<Vec<_>>(),
        vec![Type::Text, Type::I64, Type::I64]
    );
    assert!(q.columns[2].nullable);
    assert!(!q.columns[0].nullable);
}
#[test]
fn joins_ctes_aggregates_and_windows() {
    let q=check(&schema(),"WITH recent AS (SELECT id, title FROM editions WHERE version > $1) SELECT e.id, c.score, row_number() OVER (PARTITION BY e.id ORDER BY c.score DESC) AS position FROM recent AS e LEFT JOIN comments AS c ON c.edition_id = e.id",Postgres).unwrap();
    assert!(q.columns[1].nullable);
    assert_eq!(q.columns[2].ty, Type::I64);
    check(&schema(),"SELECT edition_id, count(*) AS total, sum(score) AS score FROM comments GROUP BY edition_id HAVING count(*) > $1",Postgres).unwrap();
}
#[test]
fn writes_and_queue_syntax() {
    let q=check(&schema(),"INSERT INTO editions (id, slug, title, body, published_at) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (slug) DO UPDATE SET title = excluded.title RETURNING id, version",Postgres).unwrap();
    assert_eq!(q.parameters[0].ty, Type::Uuid);
    assert!(q.parameters[3].nullable);
    check(&schema(),"UPDATE editions SET title = $1, version = version + 1 WHERE id = $2 AND version = $3 RETURNING version",Postgres).unwrap();
    check(
        &schema(),
        "DELETE FROM editions WHERE id = $1 RETURNING id",
        Postgres,
    )
    .unwrap();
    check(
        &schema(),
        "SELECT id FROM editions ORDER BY published_at LIMIT $1 FOR UPDATE SKIP LOCKED",
        Postgres,
    )
    .unwrap();
}
#[test]
fn rejects_unknown_and_ambiguous_columns_and_syntax() {
    for sql in [
        "SELECT missing FROM editions",
        "SELECT * FROM editions",
        "SELECT id FROM editions AS e JOIN comments AS c ON c.edition_id = e.id",
        "SELECT title FROM editions WHERE version = $1 AND slug = $1",
        "SELECT title FROM editions; DELETE FROM editions",
        "UPDATE editions SET missing = $1",
        "SELECT id FROM editions WHERE unknown(id) = $1",
        "INSERT INTO editions (title) VALUES (23)",
        "SELECT id FROM editions WHERE version = $2",
        "SELECT id FROM editions WHERE title = $1 UNION SELECT id FROM editions",
    ] {
        assert!(check(&schema(), sql, Postgres).is_err(), "accepted {sql}")
    }
}
#[test]
fn local_dialect_and_enums() {
    let mut s = Schema::default();
    s.apply(
        "CREATE TABLE editions (id TEXT PRIMARY KEY, title TEXT NOT NULL);",
        Sqlite,
    )
    .unwrap();
    check(&s, "SELECT id FROM editions WHERE title = $1", Sqlite).unwrap();
    assert!(check(&s, "SELECT id FROM editions FOR UPDATE SKIP LOCKED", Sqlite).is_err());
    assert!(
        s.apply("CREATE TYPE state AS ENUM ('draft');", Sqlite)
            .is_err()
    );
    let mut s = schema();
    s.apply("CREATE TYPE state AS ENUM ('draft', 'public'); ALTER TYPE state ADD VALUE 'archived'; ALTER TABLE editions ADD COLUMN state state;",Postgres).unwrap();
    assert_eq!(s.enums["state"].len(), 3);
    assert!(s.apply("DROP TYPE state;", Postgres).is_err());
}
#[test]
fn reports_line_and_bounds_recursion() {
    let mut s = schema();
    let err = s
        .apply(
            "\n\nCREATE VIEW surprise AS SELECT title FROM editions;",
            Postgres,
        )
        .unwrap_err();
    assert_eq!(err.line, 3);
    let sql = format!(
        "SELECT {}id{} FROM editions",
        "(".repeat(80),
        ")".repeat(80)
    );
    assert!(check(&s, &sql, Postgres).is_err());
}

#[test]
fn rejects_writes_to_ctes_and_untyped_parameters() {
    for sql in [
        "WITH x AS (SELECT id FROM editions) UPDATE x SET id = $1",
        "SELECT id FROM editions WHERE $1 IS NULL",
    ] {
        assert!(check(&schema(), sql, Postgres).is_err(), "{sql}");
    }
}

#[test]
fn primary_key_nullability_respects_dialect() {
    let mut pg = Schema::default();
    pg.apply(
        "CREATE TABLE records (id UUID, CONSTRAINT pk PRIMARY KEY (id));",
        Postgres,
    )
    .unwrap();
    assert!(!pg.tables["records"].columns["id"].nullable);
    let mut local = Schema::default();
    local
        .apply("CREATE TABLE records (id TEXT PRIMARY KEY);", Sqlite)
        .unwrap();
    assert!(local.tables["records"].columns["id"].nullable);
    assert!(
        local
            .apply("ALTER TABLE records ALTER COLUMN id SET NOT NULL;", Sqlite)
            .is_err()
    );
}
