# Level 4 durable-work increment — 21 October 2026

L3 PRs #5 and #6 were reviewed and merged in order. The L4 increment remains a
feature PR; no public-use or completed-Level-4 claim is made.

## Implemented

- Owned durable queue on checked PostgreSQL/SQLite SQL. Typed versioned payloads,
  mandatory idempotency declaration, transactional enqueue, stable deduplication,
  one-statement PostgreSQL claim, generation/expiry fencing, renewal, retry with
  jitter, exhaustion, incompatible payload parking and explicit queue operations.
- Owned SQL checker now resolves uncorrelated, single-column scalar SELECTs for
  the atomic PostgreSQL claim. Correlated SELECTs, multi-column scalar results,
  wrong types, unknown columns and nested writes remain rejected. A regression
  corpus seed and separately tracked compiler tests exercise this addition.
- Registry owner-scoped manifest upload and status routes. Submission locks the
  Version and enqueues in one authenticated service transaction. The upload freezes
  that draft against changes/deletion. Conflicting, foreign-owner, anonymous,
  stale-version, CSRF-less and oversized submissions are denied.
- ValidateManifestV1 records fixed finding codes; valid metadata creates a unique
  publication row. Domain effects and job acknowledgement commit together after
  checking the active claim. Duplicate or expired deliveries cannot publish again.
- A real child process claims a durable job and is killed. The replacement worker
  reclaims after expiry and produces exactly one publication record. Another test
  forces rollback after the domain insert and enqueue and checks both are absent.
- The worker has `work` and `work --once` modes. SIGINT/SIGTERM stops new claims.
  Queue list/inspect omit payloads; retry/park are explicit local operations.

## Manifest contract

The structural checks are based on Omarchy's `bin/omarchy-plugin-validate`
([source](https://github.com/omacom/omarchy/blob/quattro/bin/omarchy-plugin-validate)),
blob `728053f30175f0eafd73e5c1fdf887d37b73a3ea`, inspected 21 October 2026: schemaVersion=1, id, name, version, kinds, entryPoints,
kind-to-entry-point mapping, reserved namespace and bar default-section values.
AoR also bounds strings/counts, requires a matching semantic version and rejects
control characters, backslashes and noncanonical path components. Unknown kinds
fail closed until this validator is updated. Optional metadata is retained but
not executed.

This endpoint accepts manifest text only. It does not fetch a repository, unpack
archives, prove entry-point files exist, inspect symlinks, execute QML, scan code,
or attest installation safety. "Published" means a private Registry metadata
record passed these checks, not marketplace acceptance or public exposure.
Production uploads remain subject to the framework's existing release gates.

## Session revalidation found during review

Token creation previously rechecked the account epoch but could reuse a captured
principal after its session was logged out or expired. It now validates the
session inside the token-insertion transaction. A regression test fails on the
old implementation and passes with the fix; SQLite and PostgreSQL run it. This
adds a new tracked test file without changing existing account-denial tests.

## Remaining Level 4 work

Production mail must be enqueued atomically with account tokens and delivered
through a real adapter with provider idempotency. The current private development
mail spool remains unchanged and forbidden in production. Job scaffolding,
generic worker registration, artifact/package validation, long-running handler
heartbeat orchestration and the real Omarchy reference deployment remain open.

## Validation

See the PR's commit-bound Foundation and Database integration checks. Queue and
publication matrices run on SQLite locally and in dedicated PostgreSQL 16 CI
databases. Local AF_UNIX support remains restricted; upstream CI requires that
unchanged transport test. New boundary inventory entries cover the queue,
publication and scalar-SELECT tests; all pre-existing entries are unchanged.
No long fuzz campaign, performance benchmark, independent security review or
reference-machine acceptance is claimed.
