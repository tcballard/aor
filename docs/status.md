# Implementation status — 30 September 2026

**PR2 → PR3 → PR4 is merged into main. Level 3 account and typed-boundary foundations are implemented in this PR. This is not the completed v0.3 framework or a public-release clearance. Formal reference-Omarchy and long-running fuzz exit gates remain outstanding.**

## Built and exercised

- `aor-http`: no-allocation borrowed header parsing, strict framing and host checks, incremental request streaming, chunked response streaming, bounded keep-alive, Expect handling, Tokio TCP/Unix listeners, connection semaphore, bounded graceful shutdown.
- Fixing Everything archive: database-backed index and edition pages, transactional JSON import, embedded ordered migrations, PostgreSQL and SQLite checked queries; loopback or Unix socket. A labelled fixture remains available without a database. Actual published editions have not been supplied/imported.
- `aor-router`: segment radix tree, conflicts naming both handlers, compile-time route grammar, generated named path structs with explicit field types, sealed Path/Query/Form/Json/Body/RequestId extractors in arbitrary order, structured request traces and fixed before-route/after-handler slots, stable errors and request IDs, security headers. Cookies and bearer credentials explicitly fail before the handler.
- `aor-tmpl`: derived Rust context schemas, runtime type checking, escaped text interpolation, bool conditions, lists, three filters, typed TrustedHtml from a text sanitiser, line diagnostics, dev reload and bounded evaluation.
- Theme adapters for hexadecimal `colors.toml`, Kitty and Alacritty palettes, Tokyo Night fallback, CSS endpoint. Whole upstream theme coverage has not been established.
- `aor-dev`: direct inotify through nix; debounce, editor replacement events, recursive directory reconciliation, overflow recovery; last-good server preserved on a failed Rust rebuild, visibly labelled stale; browser reload SSE in development.
- `aor-sql`, `aor-db`, `aor-migrate`: owned DDL/query grammar, migration-derived Rust schemas, offline typed queries, PostgreSQL/SQLite pools, prepared caching, Tx, request query counts, checksums and migration locks. The deliberately missing-column query fails a downstream build.
- Embedded release CSS has a content-hashed URL and immutable cache headers. Development keeps live asset reads.
- CLI: verify, doctor, routes, dev; archive migrate/import commands. Other proposed commands are absent, not no-op stubs.
- Foundation verifier: AST escape-hatch call sites, test-file hash inventory, forbidden dependency check, source-bound fuzz-hour status, explicit incomplete framework and review gates. It does **not** implement route-to-policy call-graph analysis or the authenticated-resource negative matrix.
- Fuzz targets: requests, chunk decoder, header tokens and template parser and the SQL/DDL compiler. The initial interrupted run is not counted as successful evidence.

## Deliberate current limitations versus the specification

| Area | Remaining work |
| --- | --- |
| Level 0 exit | >=200 crash-free CPU-hours per HTTP target on current source, run behind Caddy on the actual Omarchy reference box; performance measurements. Unix socket tests have passed in unrestricted GitHub CI |
| Route API | Real session/CSRF middleware arrives with Level 3 (reserved extractors deny access now); current segment radix has no path compression. Named path types use explicit route_path! declarations rather than guessing domain types from parameter names |
| Templates | Rust-emitting release compiler; includes, inheritance, blocks, enum match, date/timezone and form helpers; attribute-safe typed outputs; current grammar rejects unsupported tags and dynamic attributes/scripts/styles |
| Dev loop | General generated apps and richer browser build diagnostics; current command runs the archive and watches top-level Cargo files/config |
| Level 2 | Implemented compiler/runtime and archive integration. Database CI runs PostgreSQL 16 transaction/migration tests and archive import/restart smoke tests; local SQLite and compile-fail checks are runnable without a database server. Reference Omarchy deployment still needs that machine. |
| Level 3 | Session persistence, passwords/tokens, CSRF, typed authorization, views, scaffolds, full verifier and negative resource matrix — not implemented |
| Level 4 | Durable jobs, transactional enqueue, lease fencing, manifest validation, mail jobs — not implemented |
| Level 5 | Arch package creation, hardened installed services, migrate unit, local starter and launcher — not implemented |
| Level 6 | Independent review, 1,000-hour parser gates, Arcade, OpenAPI, published measurements — not implemented |
| Level 7 | Embedded append log/B-tree/MVCC store and 10,000 crash cases — not implemented |

## Decisions resolved for the initial implementation

- Concrete, bounded body stream, not a public body trait. Two queued request chunks; collect enforces the configured limit. A separate completion channel prevents a late body failure being lost behind buffered chunks.
- Origin-form HTTP/1.1 plus `OPTIONS *`. Reject HTTP/1.0, absolute-form, CONNECT, TRACE, upgrades, request trailers, chunk extensions and transfer codings other than a single chunked coding. Reject all duplicated Content-Length fields, including identical values.
- The parser's accepted subset is documented in `crates/aor-http/README.md`; unsupported syntax fails closed rather than being partly interpreted.
- Rust 1.88 minimum, Linux only. PostgreSQL 16 is the tested baseline; a broader support window, reference Omarchy hardware and independent reviewer remain undecided.
- Name check before the first local commit: crates.io API returned 404 for `aor` and `aor-http`; no names reserved. `aor.dev` RDAP lookup returned HTTP 502, so domain availability is unknown. All workspace crates are `publish = false`.

## Repository publication

The supplied repository was empty. Tom explicitly authorized the documentation-only main-branch bootstrap and implementation PR on 30 September 2026. The terminal has no GitHub push credentials, so the connected GitHub API publishes the same bootstrap files and implementation tree. Local commit hashes differ from the API-created remote commits. PR1 was merged as `25dbf43`. Further implementation uses stacked feature PRs; PR2 adds the Level 2 compiler/runtime foundation; PR3 adds the database-backed archive; PR4 completes typed routing and adds compiler hardening, hashed assets and SQL fuzz coverage. See the Actions runs for exact commit-bound validation.

## Level 3 account foundation

Added persisted accounts, Argon2id passwords, sessions, verification/reset tokens, scoped API tokens, CSRF, typed owner scopes and transaction revalidation. Public-only routers retain credential denial; configured routers authenticate before hooks and handlers. Registry resources, scaffold and route-policy verifier integration follow in the next stacked PR. Current Argon2 parameters are a baseline, not reference-machine calibration.
