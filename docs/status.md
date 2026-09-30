# Implementation status — 30 September 2026

**This is a development foundation, not a completed v0.3 framework. Neither Level 0 nor Level 1 has met its complete exit gate.**

## Built and exercised

- `aor-http`: no-allocation borrowed header parsing, strict framing and host checks, incremental request streaming, chunked response streaming, bounded keep-alive, Expect handling, Tokio TCP/Unix listeners, connection semaphore, bounded graceful shutdown.
- Fixing Everything archive: database-backed index and edition pages, transactional JSON import, embedded ordered migrations, PostgreSQL and SQLite checked queries; loopback or Unix socket. A labelled fixture remains available without a database. Actual published editions have not been supplied/imported.
- `aor-router`: segment radix tree, conflicts naming both handlers, compile-time route grammar, typed path conversion, query/JSON extraction, stable errors and request IDs, security headers. Cookies and bearer credentials explicitly fail before the handler.
- `aor-tmpl`: derived Rust context schemas, runtime type checking, escaped text interpolation, bool conditions, lists, three filters, typed TrustedHtml from a text sanitiser, line diagnostics, dev reload and bounded evaluation.
- Theme adapters for hexadecimal `colors.toml`, Kitty and Alacritty palettes, Tokyo Night fallback, CSS endpoint. Whole upstream theme coverage has not been established.
- `aor-dev`: direct inotify through nix; debounce, editor replacement events, recursive directory reconciliation, overflow recovery; last-good server preserved on a failed Rust rebuild, visibly labelled stale; browser reload SSE in development.
- CLI: verify, doctor, routes, dev. Other proposed commands are absent, not no-op stubs.
- Foundation verifier: AST escape-hatch call sites, test-file hash inventory, forbidden dependency check, source-bound fuzz-hour status, explicit incomplete framework and review gates. It does **not** implement route-to-policy call-graph analysis or the authenticated-resource negative matrix.
- Fuzz targets: requests, chunk decoder, header tokens and template parser. The initial interrupted run is not counted as successful evidence.

## Deliberate current limitations versus the specification

| Area | Remaining work |
| --- | --- |
| Level 0 exit | >=200 crash-free CPU-hours per HTTP target on current source, run behind Caddy on the actual Omarchy reference box; Unix socket test on an unrestricted Linux host; performance measurements |
| Route API | Generated named path structs, full closed extractor signatures in arbitrary order, tracing span, extension slots, final session/CSRF middleware stack; current segment radix has no path compression |
| Templates | Rust-emitting release compiler; includes, inheritance, blocks, enum match, date/timezone and form helpers; attribute-safe typed outputs; current grammar rejects unsupported tags and dynamic attributes/scripts/styles |
| Assets | Content hashes and immutable caching; current release assets embed but are unhashed |
| Dev loop | General generated apps, top-level Cargo config changes, richer browser build diagnostics; current command runs the archive |
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
- Rust 1.88 minimum, Linux only. PostgreSQL support window, reference Omarchy hardware and independent reviewer are still undecided.
- Name check before the first local commit: crates.io API returned 404 for `aor` and `aor-http`; no names reserved. `aor.dev` RDAP lookup returned HTTP 502, so domain availability is unknown. All workspace crates are `publish = false`.

## Repository publication

The supplied repository was empty. Tom explicitly authorized the documentation-only main-branch bootstrap and implementation PR on 30 September 2026. The terminal has no GitHub push credentials, so the connected GitHub API publishes the same bootstrap files and implementation tree. Local commit hashes differ from the API-created remote commits. PR1 was merged as `25dbf43`. Further implementation uses stacked feature PRs; PR2 adds the Level 2 compiler/runtime foundation; its stacked archive PR adds the real database consumer. See the Actions runs for exact commit-bound validation.
