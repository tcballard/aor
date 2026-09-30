# Level 2 validation — 30 September 2026

## Local checks

- 43 workspace tests passed; `unix_socket_roundtrip` is excluded only in this sandbox, which denies AF_UNIX. The PostgreSQL test is explicitly ignored in portable runs and mandatory in database CI.
- Ten downstream compile-fail cases passed: invalid/duplicate route grammar, invalid template field, multiple body extractors, unknown extractor, missing SQL column, SELECT *, wrong parameter type, unsupported migration DDL with filename/line, SQLite row locking.
- Workspace formatting and Clippy with `-D warnings` passed.
- Archive SQLite smoke passed: migrate twice, transactional import, rejected invalid import without partial update, escaped body, linked edition, missing-edition 404, persistence through two restarts and upsert.
- HTTP/template/theme/SSE smoke passed. Development failed-rebuild/last-good/stale-status/recovery smoke passed.
- Development verification passed. Public verification returned 1 as required.
- SQL/DDL fuzz smoke completed 21.016423 CPU seconds crash-free with ASan; local leak checking is unavailable, so `release_eligible` is false. See `fuzz.json` and `fuzz-sql-0007.log`. Build time is not credited. Deliberate SQL corpus seeds are committed; generated coverage remains an intermediate artifact and CI uploads its corpus.

A local build directory produced corrupted ELF metadata during concurrent compilation. Rebuilding serially under `/tmp/aor-build` resolved it; no source test or compiler diagnostic was waived. CI uses normal clean checkouts and target paths.

## GitHub checks already completed

- PR2 at `13ec72c2a8d5d11a869107296c5c33564256068a`: [foundation run 36696464059](https://github.com/tcballard/aor/actions/runs/36696464059) and [PostgreSQL run 36696463775](https://github.com/tcballard/aor/actions/runs/36696463775) passed.
- PR3 at `9169e839be248e68b4d3aad61d00a4fec2e45fdf`: [foundation run 36696723993](https://github.com/tcballard/aor/actions/runs/36696723993) and [PostgreSQL/archive run 36696724157](https://github.com/tcballard/aor/actions/runs/36696724157) passed.

These include the Unix transport test, full sanitizer smoke, real PostgreSQL 16 migrations/checked queries/transaction cancellation, and the archive's PostgreSQL import/render/restart path. PR4 has its own commit-bound Actions checks; the above runs do not certify later source changes.

## Inventory review

The boundary inventory adds the SQL compiler, database runtime and typed-router integration tests. Existing HTTP and public-boundary test hashes remain unchanged. New denial coverage includes malformed/duplicate typed inputs, unavailable principals, hooks that cannot convert denials, security-header replacement, response-header overflow, CTE writes, untyped SQL parameters and SQLite primary-key nullability. No existing denial assertion was removed.

SQLite's initial, not-yet-merged migration now spells out NOT NULL for TEXT UUID primary keys: SQLite otherwise permits NULL in that declaration. A disposable database created from an earlier PR revision must be recreated; an applied checksum is never silently accepted after editing its migration.

## Outstanding external gates

No actual Omarchy reference host was available. No Caddy/reference-machine result, benchmark, 200-hour campaign, 1,000-hour campaign or independent review is claimed. Actual published Fixing Everything editions were not supplied; only clearly labelled fixtures were imported into disposable test databases. Levels 3–7 remain outside this implementation.
