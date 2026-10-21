# Durable jobs

Replaces Underway and apalis. AoR owns the database queue, typed versioned
payloads and lease fencing. Enqueue takes `&mut Tx`, so the domain write and job
commit or roll back together. Payloads and arbitrary exception messages are never
included in queue inspection output.

Implement `Job` with a stable `NAME`, positive `VERSION`, mandatory `IDEMPOTENCY`
and a stable `key()`. The `(job_type, dedup_key)` pair is unique for the lifetime
of the retained row. Different payloads must use different keys. A declaration
is a handler contract, not a proof of idempotency: outcome tests remain required.

`enqueue_in` checks payload (64 KiB), key, queue, attempts and time bounds.
PostgreSQL claims with one checked `UPDATE ... WHERE id = (SELECT ... FOR UPDATE
SKIP LOCKED LIMIT 1) RETURNING ...`. SQLite uses a checked select and conditional
update in the same transaction. A preceding sweep marks expired final attempts
failed. Independent SQLite connections can encounter SQLite contention and must
retry; no worker treats a failed claim as ownership.

Every claim increments a generation. `fence_in` locks the job row before domain
effects; `complete_in` runs in that same transaction. Expired, parked or replaced
claims cannot fence, renew, acknowledge or report failure. The Registry handler
commits publication, validation findings and acknowledgement together. External
effects need their own provider idempotency key; this crate does not promise
exactly-once network delivery.

`renew` is the heartbeat primitive for long-running handlers. The current Registry
handler makes no external calls and processes a bounded manifest inside its
fenced transaction. Workers stop claiming on SIGINT/SIGTERM, finish the current
bounded job, and leave a killed process's lease for recovery.

Failures retain only framework error codes. Retries use exponential delay plus
per-job jitter, capped at one hour. Final attempts fail; explicit requeue resets
the budget. Unknown payload versions park as `incompatible` without consuming an
attempt. The Registry worker registers only `registry.validate-manifest` v1.
Job scaffolding and the transactional mail worker are implemented. Scheduling remains outside this level.

Run `cargo test --locked -p aor-jobs`. The PostgreSQL test additionally requires
an isolated `AOR_TEST_DATABASE_URL` and `-- --include-ignored`; CI creates its own
disposable database.
