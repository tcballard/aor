# L3 implementation record — 30 September 2026

PR2–PR4 were merged into main at `d34feaeeb1af5d2cdc1c0fb149308c51ade6e513`.
The new work is split into persisted accounts/typed boundaries and the Registry,
scaffold and verifier integration. No L3 PR is automatically merged.

## Second-resource brief

“Add versions beneath plugins: each version belongs to its creator and an existing
plugin owned by that same account. Support private listing, reading, creation,
versioned editing and deletion, denying other owners, foreign parents and ownership
or parent reassignment.”

The same implementation agent completed this exercise; it is not an independent
agent ergonomics study. The initial implementation was written against the typed
boundary, then the actual scaffold command generated Version's contract, migration
and incomplete test file. The final implementation and HTTP matrix completed that
contract; the scaffold-created `003_versions.sql` is retained. No fabricated passing
matrix stubs were used. Dry-run and creation were exercised, and existing-file
conflicts were rejected. Generated pre-completion SHA-256 values:

- `src/versions.rs`: `a29c2d955a6ffc24619c419f950742414efb6ff2fb6d02d1c3415e5329db222b`
- `migrations/003_versions.sql`: `7ab1e294e27a06b624ec19d1cab8b3ff4dd0857e3ed9f169ac6cd380ecd2830c`
- `tests/versions_matrix.rs`: `3e4dbca92ec8927b6063ed4254b962ccb3e7b247e9cb19fed83f6761d41fcfe3`

## Local validation

- Account persistence/replay/CSRF/expiry/revocation suite: passed on SQLite.
- Plugin and Version denial matrix: passed through the owned HTTP transport over
  Tokio duplex connections, with SQLite persistence.
- Workspace tests: passed excluding `unix_socket_roundtrip`, which fails with the
  environment's EPERM on socket creation. No CI skip was added.
- Formatting and clippy are required before publication. Compile-failure cases
  cover forged principals/scopes and client-supplied ownership in addition to L2 cases.
- Existing protected test hashes were checked against committed bytes before
  adding the three new suites. No previous denial test or hash was changed.
- PostgreSQL 16 account and Registry matrices run in separate fresh CI databases;
  local SQLite results do not substitute for those checks.

## Honest outstanding gates

Argon2id uses m=19456 KiB, t=2, p=1 as an explicit initial baseline. It has not been
calibrated on the reference Omarchy machine. No long fuzz hours, reference Caddy
run, independent review or production mail delivery is claimed. Version publication
and duplicate job delivery are L4 work; root plugins have no parent case. The
verifier supports the direct generated route/service shape and does not claim a
sound whole-program Rust call graph. Public readiness remains false.
