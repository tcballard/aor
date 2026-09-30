# Owned SQL compiler (Level 2)

No database is contacted by the compiler. `aor-migrate::build("migrations",
Dialect::Postgres)` derives `OUT_DIR/schema.rs`. The `sql!` macro uses the same
schema deriver, tracks each migration with `include_str!`, and generates named
query/row types. The build script tracks the directory, including new files.

```rust,ignore
aor_db::sql!(pub ListEditions, postgres, "migrations",
    "SELECT id, title FROM editions WHERE slug = $1 LIMIT $2");
let rows = ListEditions::query(&mut tx, slug, 20_i64).await?;
```

This named-query form deliberately exposes stable Rust row types rather than an
anonymous structural type. Parameters are owned Rust values. Nullable INSERT or
UPDATE parameters are `Option<T>`. Results preserve column nullability and make
right-hand columns of LEFT JOIN nullable. `execute` returns affected rows.

## Grammar

Unquoted ASCII identifiers; SQL strings with doubled quotes; numbered `$1`
parameters; `--` comments. One query per invocation. Migrations require semicolons.
Limits: 1 MiB SQL, 65,536 tokens, 256 parameters, bounded expression/CTE nesting.

DDL: CREATE/DROP TABLE, CREATE/DROP INDEX (optionally UNIQUE or CONCURRENTLY),
CREATE/ALTER/DROP TYPE ENUM; ALTER TABLE ADD/DROP/RENAME COLUMN, RENAME TO,
ALTER COLUMN TYPE/SET/DROP NOT NULL/DEFAULT, ADD/DROP named constraints.
Constraints: PRIMARY KEY, UNIQUE, REFERENCES, FOREIGN KEY, and CHECK comparisons
of a column to a literal. Defaults: literal, NULL, NOW(), CURRENT_TIMESTAMP.
Types: boolean, int4/integer, int8/bigint, text, bytea/blob, uuid, timestamptz,
timestamp with time zone, declared enums. Floating point and NUMERIC are excluded;
monetary values use integer minor units.

Queries: SELECT explicit expressions FROM a table/CTE; aliases use AS; INNER/LEFT
JOIN ON; non-recursive WITH; WHERE, GROUP BY, HAVING, ORDER BY, LIMIT/OFFSET;
INSERT one VALUES tuple with ON CONFLICT DO NOTHING/DO UPDATE; UPDATE, DELETE;
RETURNING; PostgreSQL FOR UPDATE [SKIP LOCKED|NOWAIT]. Expressions: qualified or
unqualified columns, parameters, literals, arithmetic, comparisons, AND/OR/NOT,
IS [NOT] NULL, LIKE; COUNT, SUM(INT4), MIN/MAX, LOWER/UPPER, two-argument COALESCE;
ROW_NUMBER/RANK/DENSE_RANK and OVER (PARTITION BY / ORDER BY).

SQLite migrations use TEXT for UUIDs and UTC timestamps. Its checker rejects
enums and row locks. SQLite's actual DDL restrictions are also enforced when
migrations run. Unsupported SQL fails with a location and `sql_unchecked!` guidance.
No SELECT *, implicit casts, quoted identifiers, schema-qualified tables,
recursive CTEs, subqueries, window frames, expression indexes or arbitrary functions.
The checker resolves schema/types; database constraints and planner restrictions
(e.g. GROUP BY functional dependencies) remain database-authoritative.

`sql_unchecked!` is a visible, verifier-reported escape hatch, not an alternate
query builder. It returns runtime rows and takes explicit dialect and values.
