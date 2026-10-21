# Level 4 mail and job scaffolds — 21 October 2026

PR #7 is merged. This follow-up implements the remaining mail path and fail-closed
job scaffolding; it does not clear the public, independent-review, reference-host
or long-fuzz gates.

## Account mail

Registration/password reset now write their hashed token, an outbox entry and a
job inside the same transaction. Production Registry startup requires an outbox;
the legacy private spool stays development-only. Public responses keep the same
existence-concealing shape. Reset endpoints without any delivery adapter refuse
before looking up an account.

The job contains only a delivery UUID. The worker reconstructs the token with a
protected signing key and RustCrypto HMAC-SHA256; stored outbox/account records
contain token digests. There are no serialized raw delivery tokens, API keys or
signing keys in the database, queue inspection or fixed-code mail errors.

The Resend adapter uses the system curl executable with a fixed HTTPS endpoint,
no redirects/config inheritance, secrets on stdin and bounded execution. No
provider credentials have been supplied or used in this work. Tests use fictional
addresses and controlled doubles/loopback fixtures, including real curl request
encoding. They do not demonstrate live API authentication or inbox delivery.

## Worker guarantees

Mail renews its lease during external work. Losing the lease cancels the transport
future; an expired generation cannot acknowledge. A provider-accepted request with
a lost response retries the same immutable body/key and the outcome test observes
one provider effect. A 23-hour replay guard is deliberately shorter than the
provider's documented 24-hour window. Ambiguous old jobs fail without sending,
even after explicit requeue. Expired messages never reach the transport.

The Registry runtime alternates the Registry/mail queues when configured. Signals
cancel new claim attempts, drain the current handler for at most 30 seconds, then
drop remaining work. Provider calls are bounded and canceled curl children are
killed. Payload versions outside the explicit registered versions park unchanged.

## Scaffold contract

`cargo aor scaffold job Name --root APP [--dry-run]` emits a versioned payload,
stable job name, mandatory idempotency declaration, handler boundary and duplicate
outcome test. The result fails compilation until these are implemented. It never
chooses a passing idempotency strategy or supplies a no-op handler. Dry-run returns
the exact file contents; apply refuses conflicts and symlink destinations.

CLI tests also exercise non-overwriting private key generation, read-only help,
unknown-job parking and clean SIGTERM shutdown of the real worker process.

The compile-failure harness checks an actual generated fixture. It compares
preview/applied file bytes and requires `AOR_JOB_INCOMPLETE`. Other cases reject
detached mail enqueue and wire serialization of the issued-token type.

## Validation and limits

SQLite tests cover account/outbox/job rollback, unknown-account concealment,
verification/reset success and single use, expired/wrong-key denial, lost provider
responses, bounded replay, lease renewal/cancellation and private-key files.
PostgreSQL CI runs the account/outbox/replay matrix in its own disposable database.
The new test files are added to the boundary inventory; old hashes are unchanged.

Actual Omarchy deployment, real provider credentials/verified sender acceptance,
password calibration, package contents, independent security review and long fuzz
campaigns remain outside these results. Multi-key rotation and delivery webhooks
are not implemented. A `delivered` outbox entry means the provider accepted the
request, not that a user received or read it.
