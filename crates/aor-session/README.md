# aor-session

Persisted server-side accounts, sessions, CSRF and scoped API tokens. `Auth::new`
requires a validated public origin and cookie configuration. Production refuses an
insecure origin/cookie. Public-only routers still reject credentials; opt in with
`Router::with_auth(auth)` only alongside policies and service transaction scopes.

Tokens contain 256 random bits from getrandom and are stored as SHA-256 hashes.
Passwords use RustCrypto Argon2id (19 MiB, 2 iterations, 1 lane) on a bounded blocking
pool; reference-machine calibration remains outstanding. `Secret`, `Session` and
`Principal` have no wire serialization or Debug implementation. Only mail adapters
receive verification/reset deliveries. HTTP adapters must return generic responses
for registration and reset requests and never expose delivery tokens.

`Auth::migrate` is for an auth-only migration history. An app with domain tables must
include `migrations/001_auth.sql` as its initial migration and run its complete
combined history instead. The shared PostgreSQL/SQLite schema stores canonical UUID
text and UTC epoch seconds. `with_clock` injects a trusted clock for expiry tests.

Cookie mutations require exact Origin, compatible Sec-Fetch-Site when present, and
matching CSRF cookie/header or validated form `_csrf`. Bearer tokens have explicit
resource/action scopes; mixing them with a session cookie is rejected. Account
administration requires a session principal. `aor_tx::begin` revalidates the account
epoch, credential and expiry in the domain transaction before repository effects.

Tests cover login rotation, token reuse/revocation, expiry, CSRF, configuration and
account-wide revocation. PostgreSQL tests require their own empty disposable database
and `--include-ignored`. Independent security review has not been completed.
