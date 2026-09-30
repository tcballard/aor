# Database runtime

`Pool::postgres` uses tokio-postgres on loopback or Unix sockets (NoTls); remote
hosts require a separately designed TLS transport and are rejected. `Pool::sqlite`
uses bundled SQLite, foreign keys and WAL with one serialized lease. Pool sizes,
acquisition deadlines, query deadlines and prepared-statement caches are bounded.

`let mut tx = lease.begin().await?` borrows one connection. All checked query types
accept either a lease or `&mut Tx` through Executor. Commit/rollback consume Tx.
Dropping unfinished work or cancelling a query poisons/discards the connection;
it is never returned to the idle pool. PostgreSQL closing rolls back at the server.
SQLite executes on blocking workers; disposal rolls back after the worker exits.
A timed-out SQLite statement may finish before its connection closes, so callers
must use transactions for writes that require rollback on cancellation.

`QueryCounter::scope(future)` records per-request SQL fingerprints/counts without
parameter values. `repeated(threshold)` exposes possible N+1 queries to development
handlers. It is task-local and does not silently combine concurrent requests.

Ordered migrations use `VERSION_name.sql`. Checksums include exact file bytes.
PostgreSQL uses a session advisory lock; SQLite uses BEGIN IMMEDIATE. Transactional
migrations record history atomically. `-- aor: non-transactional` on the first line
explicitly opts PostgreSQL migrations out; dirty history is written first, and an
interrupted run fails closed until an operator repairs it. Never edit applied files.

As with either underlying database, cancellation or transport failure during
COMMIT can leave the outcome uncertain. Connection disposal prevents reuse; it
cannot undo a commit already accepted by the database. Do not assume an error
means the transaction was not committed.

`sql!(Name, portable, ...)` checks a shared migration/query against both PostgreSQL and SQLite and requires identical parameter/result Rust types. It selects the executor dialect at runtime. Use explicit dialects for native UUID/timestamp schemas or backend-specific SQL.
