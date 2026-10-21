# Level 4 reusable worker runtime — 21 October 2026

The queue, manifest metadata publication and account outbox were already merged
in PRs #7 and #8. This increment closes the reusable-worker implementation gap
without changing their migrations or weakening their denial tests.

## Runtime

`aor_jobs::Worker` registers typed `Handler<J>` implementations under an explicit
queue, stable job name and exact payload version. Duplicate registrations fail;
unregistered names/versions/queues park without consuming attempts. Multiple
versions can coexist. Decoding happens before invoking the handler. Queue polling
rotates after each attempt so a busy manifest queue cannot starve account mail.

The runtime supervises lease renewal and bounded shutdown. A handler that holds
a fenced transaction continues being polled while renewal waits for that same
row/connection, rather than deadlocking on itself. Renewal errors or deadlines
cancel its future. Shutdown stops claims, drains current work while renewing,
and drops it after the drain window. A dropped database transaction rolls back;
its unfinished job remains recoverable after lease expiry.

Handlers retain ownership of the fenced domain transaction and acknowledgement.
The runtime never acknowledges just because a callback returned success. Failed
handlers receive fixed queue error codes and bounded retries; a handler that
already disposed its claim is not retried again. Debug and Display both redact
underlying database details. External effects still require provider idempotency;
dropping a future cannot undo a provider-accepted request. Non-yielding synchronous
handlers cannot be preempted by an async deadline.

Registry now builds this runtime with manifest and optional mail handlers instead
of owning another dispatch/poll/drain loop. The standalone mail worker retains
its independent transport heartbeat. Generated jobs emit the typed Handler shape
and still fail compilation until idempotency, effects and denial outcomes exist.

## Fault-injection matrix

`crates/aor-jobs/tests/worker.rs` adds actual outcome assertions for:

- Multiple accepted versions, unknown versions, malformed payloads, duplicate
  registration, queue isolation and alternating queue service.
- One domain effect under repeated claim processing; rollback before retry and
  retry exhaustion with no committed effect.
- Heartbeat extension past the initial expiry and lease-loss cancellation before
  effects, followed by a replacement's single successful effect.
- A handler holding its own queue lock across heartbeat ticks without deadlock.
- Successful drain without claiming the next job; immediate stop before claiming.
- Drain timeout while a domain insert is uncommitted, followed by rollback and
  recovery with exactly one effect.
- A real child process killed after its fenced domain insert and before commit;
  both backends must roll back, recover the expired lease and commit one effect.

This is a new tracked test file. Existing boundary-test hashes are unchanged.
The compile-failure harness additionally rejects registering a handler for a
payload it does not handle. Generated job denial still uses the real scaffold.

## Reproduction and acceptance limits

Run `cargo test --locked -p aor-jobs --test worker` for portable SQLite outcomes.
PostgreSQL uses the same matrix with `postgres_worker_fault_matrix -- --include-ignored`
and an isolated `AOR_TEST_DATABASE_URL`; CI provisions the separate `aor_worker`
database. Existing Registry publication/mail and worker process tests remain
required. Run the development verifier and all compiler-denial fixtures too.

No live email credentials, real messages, Omarchy deployment, independent review,
reference-machine calibration, package inspection or long fuzz campaign are
claimed. Those external acceptance and public-release gates remain open.
