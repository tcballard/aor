# AoR contributor reference

Read `docs/status.md` first, then `docs/specification-v0.3.md`. AoR is an acronym only.

## Boundaries

- Do not describe this foundation as the completed framework or safe for public traffic. `verify` explicitly reports unimplemented gates.
- Do not replace owned layers with hyper, axum, tower, an ORM, a template engine or a watcher crate. Tokio, serde, cryptographic primitives and wire protocols are the bought foundations; see the specification.
- Do not accept cookie or bearer credentials as authenticated. This milestone rejects them before handler invocation. Implement the session, CSRF and policy boundary together before admitting protected routes.
- Do not bypass HTTP framing checks. The transport owns Content-Length, Transfer-Encoding and Connection response headers.
- Do not add raw HTML constructors. Use the named sanitiser and `TrustedHtml`; plain strings are escaped.
- Do not weaken or delete denial tests to make a change green. Review any `.aor/boundaries.json` changes explicitly with the corresponding test diff.
- Do not claim fuzz hours from compilation, wall time, a different source hash, failed runs, or unrecorded campaigns. `scripts/fuzz.py` records child CPU time and source hashes.
- Do not add a pretend implementation of an unbuilt level. Add the real behavior and denial tests, then update `docs/status.md`.
- Do not merge or push to main without authorization. Use feature branches and PRs after repository bootstrap.

## Directory map

| Path | Purpose |
| --- | --- |
| `crates/aor-http` | Byte parser, incremental chunk decoder, bounded TCP/Unix server |
| `crates/aor-router` | Public routes, path/query/JSON extraction, error mapping |
| `crates/aor-tmpl` | Interpreted, schema-checked templates and theme adapter |
| `crates/aor-sql`, `crates/aor-db-macros` | Owned SQL/DDL grammar and offline checked query expansion |
| `crates/aor-db`, `crates/aor-migrate` | Database pools, transactions, migration execution and schema generation |
| `crates/aor-macros` | Compile-time route grammar and context-schema derive |
| `crates/aor-dev` | Inotify and last-good development process |
| `crates/aor-verify` | Foundation integrity and release-evidence reports |
| `crates/aor-cli` | `cargo aor` commands implemented so far |
| `crates/aor` | Umbrella exports |
| `apps/archive` | Running Fixing Everything development fixture |
| `tests/boundaries` | Mandatory negative tests, not the future full resource matrix |
| `fuzz` | Instrumented targets and committed corpora |
| `docs/evidence` | Actual results and source-bound fuzz time |

## Exact commands

Run from the repository root, with Rust 1.88 or newer and Cargo on PATH.

| Command | Expected exit |
| --- | --- |
| `cargo build --locked --workspace` | 0 |
| `cargo test --locked --workspace` | 0 on a Linux host supporting AF_UNIX |
| `cargo fmt --all --check` | 0 |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | 0 |
| `python3 scripts/smoke.py` | 0 |
| `python3 scripts/database-smoke.py` | 0; PostgreSQL also runs when an isolated AOR_TEST_DATABASE_URL is supplied |
| `python3 scripts/check-compile-fail.py` | 0 |
| `cargo aor verify --development --json` | 0 if current foundation integrity checks pass |
| `cargo aor verify --json` | 1 until public-release gates are fulfilled |
| `cargo aor routes --json` | 0 |
| `cargo aor doctor --json` | 0 with Linux/Rust/Cargo available |
| `cargo aor dev` | Runs until SIGINT/SIGTERM; 0 on normal shutdown |
| `cargo run -p aor-archive -- serve` | Runs until SIGINT/SIGTERM; loopback port 3000 |
| `python3 scripts/fuzz.py --seconds 60` | 0 only if all instrumented targets finish without failure |

Fuzzing requires `rustup toolchain install nightly --profile minimal` and
`cargo +nightly install cargo-fuzz --locked` first. Do not count install/build CPU time.

`AOR-BOUNDARY-001` means the committed denial-test inventory differs. Inspect the diff;
do not regenerate hashes as a routine repair. `AOR-FUZZ-001`, `AOR-SPEC-001` and
`AOR-REVIEW-001` are real unmet release gates. A development pass does not waive them.

## Never edit as a shortcut

No fake review attestations, CPU-hour increments, passing matrix labels, fabricated
benchmarks, production-ready labels, or blanket Unix-test skips in CI. The local
workspace denies Unix socket creation; record that environment limitation separately.

Level 2 integration targets PostgreSQL 16 in CI. Runtime PG tests are ignored in the
portable suite and explicitly required with `--include-ignored` in database.yml.
Never use a real-data database for AOR_TEST_DATABASE_URL. `sql!` uses named query
types; see crates/aor-sql/README.md. Keep migration DDL and all application SQL
checked; raw escapes must remain visible to verify. No schema-only or timeout
failure should be treated as a database test pass.
