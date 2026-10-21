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

`Worker` registers `Handler<J>` implementations with
`register::<PayloadV1>(queue, handler)`. A handler receives the decoded payload
and a `Context` containing the pool, opaque claim and injected clock. Registration
checks the stable name, version and idempotency declaration, and rejects duplicate
queue/name/version tuples. Multiple payload versions require separate typed
registrations. Dispatch includes the queue: a registration cannot handle another
queue's claims. `work_once` claims and processes one job; `run_until(stop)` polls
registered queues fairly until the supplied shutdown future resolves.

The runtime renews leases while handlers run. It keeps polling a handler while
renewal waits for the handler's own transaction lock, avoiding a self-deadlock.
A renewal failure or deadline drops the handler future. Defaults are a 60-second
lease, 10-second heartbeat, 100 ms poll and 30-second shutdown drain. Configuration
rejects an invalid lease, zero heartbeat/poll, or heartbeat longer than a third
of the lease. Shutdown cancels new claim attempts, drains current work with its
heartbeat active, then drops unfinished work so its transaction rolls back and
its lease can expire. Inject a clock with `with_clock` for fault-injection tests.

Handlers still call `fence_in` before effects and `complete_in` in the same
transaction. Returning `Ok(())` does not acknowledge on another connection. A
handler error rolls back its transaction and the runtime records a fixed failure
code if the generation is still owned. Already-disposed claims are not retried
again. Cancellation drops Rust futures; blocking/non-yielding code cannot be
preempted, and network side effects still require provider idempotency.

Registry uses this runtime for manifest and account-mail registration. Its mail
adapter also retains transport-level heartbeats for safe use outside the generic
runtime. The bounded manifest handler publishes findings and metadata in its
fenced transaction.

Failures retain only framework error codes. Retries use exponential delay plus
per-job jitter, capped at one hour. Final attempts fail; explicit requeue resets
the budget. Unknown payload versions park as `incompatible` without consuming an
attempt. The Registry worker accepts `registry.validate-manifest` v1 and, when configured,
`aor.mail.delivery` v1.
Job scaffolding and the transactional mail worker are implemented. Scheduling remains outside this level.

Run `cargo test --locked -p aor-jobs`. The PostgreSQL test additionally requires
an isolated `AOR_TEST_DATABASE_URL` and `-- --include-ignored`; CI creates its own
disposable database.

The worker fault matrix tests version dispatch, queue isolation, fairness,
malformed payloads, rollback/retry exhaustion, renewal, lease-loss cancellation,
transaction-lock progress, drain/timeout and real process death with an
uncommitted effect. SQLite and disposable PostgreSQL run the same matrix.
See [implementation evidence](../../docs/evidence/l4-worker-runtime.md).
