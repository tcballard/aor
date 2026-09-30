# Fixing Everything archive

Without database configuration, `serve` runs the original labelled development
fixture (useful for HTTP/theme tests). With a database it renders stored editions.
The supplied JSON is test content, not a claim that Tom's published archive was imported.

```sh
# PostgreSQL 16 on loopback; set credentials in the environment, not command arguments.
export AOR_DATABASE_URL='postgres://USER:PASSWORD@127.0.0.1/aor_archive'
cargo run -p aor-archive -- migrate
cargo run -p aor-archive -- import apps/archive/fixtures/editions.json
cargo run -p aor-archive -- serve --dev

# Embedded local equivalent (no PostgreSQL daemon).
cargo run -p aor-archive -- migrate --sqlite archive.sqlite
cargo run -p aor-archive -- import apps/archive/fixtures/editions.json --sqlite archive.sqlite
cargo run -p aor-archive -- serve --sqlite archive.sqlite --dev
```

Imports are arrays of `{slug,title,body,published_at}`. Slugs are canonical lower-case
ASCII; timestamps must be RFC3339 and are stored in UTC. Body is plain text and is
escaped in the template. The importer validates the whole file, then upserts all
rows in one Tx. Existing slugs retain their UUID and increment their version.
Limits: 8 MiB/file, 1,000 editions/import, 256-byte title, 256 KiB body. The index
shows the newest 100 editions; detail pages are `/editions/{slug}`.

Migrations are explicit operational commands, never an HTTP write endpoint or a
startup side effect. Schema and migration SQL embed in the release binary.
`build.rs` emits Rust schema types from PostgreSQL DDL; query macros check both
PostgreSQL and SQLite migration trees. All application reads and writes are checked.
No cookie/bearer auth or remote editing is admitted before Level 3.

`python3 scripts/database-smoke.py` checks SQLite and, with
`AOR_TEST_DATABASE_URL`, a disposable PostgreSQL database. It deletes archive test
tables in that explicitly supplied test database. Never point it at real data.
