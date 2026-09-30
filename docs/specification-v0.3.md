# AoR — product and technical specification v0.3

Sep 30, 2026 · @Tom Ballard · from-scratch edition

## 1. Thesis

Build a complete Rust web framework from the socket up — HTTP server, router, query layer, migrations, template compiler, sessions, queue, CLI and dev loop — and use it to run every web-facing thing in the Omarchy side of Tom's life. Keep the v0.2 verification core (typed authorization scopes, transaction handles, negative test matrix, `verify` report) as the spine, so what comes out is trustworthy enough to expose to the Omarchy community rather than only to its author.

This is a challenge specification. v0.2 optimised for the shortest path to a product other people would adopt; v0.3 optimises for depth of understanding, the pleasure of owning every layer, and a framework whose every default fits one operating system and one community.

Every section names the mature crate the layer replaces, so the design departures are explicit and each crate's README can open with what is done differently here.

AoR is the working name, used as a bare acronym. `cargo aor` commands and APIs are proposed, not implemented.

## 2. What changed from v0.2

| v0.2 | v0.3 | Reason |
| --- | --- | --- |
| Extension to Loco; own nothing below the boundary types | Independent framework; own every layer above the wire protocols | The goal is now the build, not the shortest route to adoption |
| Audience: any developer directing an agent | Audience: Tom, then Omarchy users and contributors | One OS and one community let every default be opinionated |
| Reference app: Release Hub | Reference apps: the Omarchy Plugin Registry, the Omarchy Arcade leaderboard, the Fixing Everything publishing site | Real things that will run in public on this framework |
| PostgreSQL only | PostgreSQL for hosted apps; an embedded single-file mode for local Omarchy apps | Omarchy apps often run on one laptop with no daemon |
| Generic HTML defaults | Generated UI reads the user's live Omarchy theme | The framework should look native on the desktop it targets |
| Stop condition: Loco passes the gates | Stop condition: a layer becomes a security liability the author cannot review | The challenge is allowed to be hard; it is not allowed to be unsafe in public |

Carried forward unchanged: the five-part verification core (section 13), the negative test matrix, `verify` as the centre of the workflow, the agent interface (`AGENTS.md`, deterministic CLI, stable rule IDs), and the principle that nothing security-relevant hides in a handler. Those were the good ideas; the runtime under them is now the author's.

## 3. The line

Everything above the wire protocols is built here. Three things are not, and the reason is the same for each: rewriting them teaches little and creates a class of bug the author cannot review alone.

| Layer | Built or bought | Replaces | Why the line falls here |
| --- | --- | --- | --- |
| Async runtime and I/O | Bought: Tokio | — | A runtime is a separate multi-year project; not the challenge |
| TLS | Bought, and kept out of the process: terminated by Caddy or nginx | rustls | Certificate handling is not a place to learn in public |
| Cryptographic primitives | Bought: RustCrypto Argon2, SHA-2, HMAC, `getrandom` | — | Never written from scratch, in any version of this spec |
| PostgreSQL wire protocol | Bought: `tokio-postgres` for the protocol only; no query builder, no ORM | — | The protocol is a spec-following exercise; the query layer above it is where the design lives |
| HTTP/1.1 parsing and serving | Built: `aor-http` | hyper | The most instructive layer and the most dangerous; fuzzed from day one |
| Routing, extractors, middleware | Built: `aor-router` | axum, tower | The place the framework's opinions live |
| Query layer, schema, migrations | Built: `aor-db`, `aor-migrate` | SeaORM, sqlx, refinery | Compile-time schema checks against migrations are the headline feature |
| Templates | Built: `aor-tmpl` proc-macro compiler | Askama, MiniJinja | Typed contexts, Omarchy theme awareness, dev-mode reload |
| Sessions, CSRF, password auth | Built: `aor-session` | tower-sessions | Small, well-specified, and where the matrix tests earn their keep |
| Durable queue | Built: `aor-jobs` | Underway, apalis | Transactional enqueue and lease fencing are the whole design |
| CLI, dev loop, file watching | Built: `aor-cli`, `aor-dev` on raw inotify | clap, watchexec | Owning inotify is fun; owning argument parsing is a chore, so `clap` stays as the one exception |
| Serialisation | Bought: serde | — | A serde replacement is a different hobby |
| Embedded local storage | Built later: `aor-store`, a single-file append log with an index | SQLite | Stretch goal; SQLite is used until it exists |

HTTP/2 and HTTP/3 are out. The reverse proxy speaks them to the client; AoR speaks HTTP/1.1 on loopback or a Unix socket. WebSockets are a level of their own after HTTP/1.1 is fuzzed clean.

## 4. Omarchy as the sandbox

Omarchy is an opinionated Arch Linux and Hyprland setup with a strong theme system, a plugin ecosystem forming around it, and a community that installs things from one person's GitHub. Every one of those properties simplifies a framework:

- One OS means one packaging target (pacman/AUR), one init system (systemd), one filesystem layout, one browser engine to test against first.
- A live theme system means generated UI can read the current theme and look native, instead of shipping a neutral stylesheet.
- A community that runs each other's tools means the security bar is real from the first public release. Nothing here is "just a local tool" once it is on a URL.

The primary user is Tom. The secondary users are Omarchy users who install apps built on AoR, and contributors who build their own. The agent remains a first-class user of the CLI and instruction file, as in v0.2.

Reference applications, in build order:

| App | What it exercises | Where it runs |
| --- | --- | --- |
| Omarchy Plugin Registry | Accounts, owner-scoped plugin and theme entries, manifest upload and validation as a durable job, public browse pages, JSON API for the installer | Hosted, PostgreSQL, behind Caddy |
| Omarchy Arcade leaderboard | Anonymous score submission with signed payloads from the games, rate limiting, per-game boards, SSE live updates in a later level | Hosted |
| Fixing Everything site | The weekly edition as a content system: drafts, publish job that renders and pushes static output, RSS | Hosted, low traffic, tests the template compiler hardest |
| Local mode demo | A single-binary Omarchy app (a task manager or a theme picker) with the embedded store, no PostgreSQL, launched from Walker | The laptop |

The registry is first because it is the one the community will use, and because it exercises every boundary the verification core guards: ownership, nested resources (plugin → versions), a transaction with a durable side effect (validation job), and an external effect that must not fire before commit (notification, later). Native Omarchy windows, shell plugins and desktop widgets remain outside the framework; they consume its HTTP APIs.

## 5. Goals, exclusions and the honesty clause

| Goal | Required outcome |
| --- | --- |
| Own every layer above the line | Each crate in section 3 is written, documented and tested by the author; no layer is a thin wrapper over the crate it replaces |
| Trustworthy in public | The verification core and the fuzzing programme make the registry safe to expose to the Omarchy community |
| Compile-time schema truth | SQL in application code is checked against the committed migrations at compile time, with no live database needed to build |
| Native on Omarchy | Generated UI adopts the live theme; apps package as pacman packages with systemd units; local mode needs no daemon |
| Agent-legible | `AGENTS.md`, deterministic CLI, `verify --json` with stable rule IDs, as v0.2 |
| Each level ships | Every milestone ends with a running Omarchy app on the framework, not a library with no consumer |
| Understandable without the CLI | Plain `cargo build` and `cargo test` always work; the CLI is convenience |

Excluded permanently: a runtime, TLS, cryptographic primitives, the PostgreSQL protocol, HTTP/2 and HTTP/3 in-process, a JavaScript framework, a hosting service, a bundled agent or model, telemetry.

Excluded from v1, revisit by demonstrated need: OIDC, organisation tenancy, WebSockets, a shared cache, an admin UI, GraphQL, Windows and macOS support (Linux only; it is an Omarchy framework).

**The honesty clause.** Three statements are true and stay in the README:

1. Every layer here has a mature crate that does it better today. AoR exists because building it is the point.
2. The HTTP parser, session handling and CSRF implementation are the author's and have had the review described in section 17 and no more. Deploy behind a reverse proxy; do not put it on the public internet without one.
3. If a security defect is found in a layer the author cannot fix within a week, that layer is replaced by the crate it displaced and the spec is amended. Pride is not a release gate.

## 6. Product principles

1. Build it, then fuzz it, then ship it. Nothing that parses bytes from the network reaches a public app before it has run under a fuzzer for a recorded number of CPU-hours with a recorded corpus.
2. The type system is the reviewer. Transaction handles, authorised scopes, view types and typed template contexts are types, not conventions. Misuse fails to compile.
3. The migrations are the schema, at compile time too. `aor-db` derives the schema from the committed migration files during the build; SQL that references a missing column is a compile error.
4. Negative before positive. Every scaffolded resource ships its denial tests first; `verify` fails if they are removed.
5. Nothing security-relevant in a handler. Policies, services and the matrix are the review surface, as v0.2.
6. One OS, so decide. Arch paths, systemd units, pacman packaging, Hyprland-adjacent conventions are defaults, not options. Portability is somebody else's project.
7. Native by default. If the user has an Omarchy theme, the app wears it. If not, it wears Tokyo Night and does not apologise.
8. Fun is a design constraint. Each level has a piece the author wants to build. If a level has none, the plan is wrong and gets reordered.
9. Name the crate you replaced. Every crate's README opens with the mature alternative and one paragraph on what is done differently here.
10. Deterministic for agents. Same inputs, same version, same output — for scaffolds, `routes`, `verify` and error codes.
11. Own as little below the line as possible, and own everything above it completely. No half-wrappers.

## 7. Architecture

&#91;embedded content: AoR crate map · 10 owned crates, 3 bought foundations\]

Requests enter from the reverse proxy, pass down through the owned layers, and touch the bought foundations only at the bottom. The verification core (`aor-policy`, `aor-tx`, `aor-verify`) sits in the middle of the owned band deliberately: it is the part that makes the rest safe to expose.

Request lifecycle, in order: `aor-http` accepts the connection, enforces per-connection and per-request limits, and parses the request line and headers into a bounded `Request`; `aor-router` matches the path, runs middleware (request ID, tracing span, session load, CSRF check on cookie-authenticated mutations), and runs typed extractors; the handler calls a policy to obtain an `Authorized<…>` scope, calls a service, and receives a `View`; the response is rendered through `aor-tmpl` or serialised, and `aor-http` writes it with correct framing and keep-alive handling. Middleware ordering is fixed by the framework and tested, not composed by the application.

Workspace layout is one crate per box above, an `aor` umbrella crate re-exporting the prelude, a starter template, and an `aor-testkit` crate for the matrix and the fuzz harnesses. Each owned crate is independently usable: `aor-http` with no router, `aor-db` with no HTTP. That constraint keeps the boundaries honest and makes each crate testable on its own.

Two processes are built from one application binary: `app serve` and `app work`. Local mode (section 16) runs both in one process with the embedded store.

## 8. aor-http

Replaces hyper. An HTTP/1.1 server on Tokio TCP and Unix sockets, designed to sit behind a reverse proxy and to be fuzzed before it is trusted.

Scope: request-line and header parsing per RFC 9112, with strict handling of the ambiguities that cause request smuggling; `Content-Length` and chunked bodies, never both; keep-alive with a bounded number of requests per connection; `Expect: 100-continue`; correct `Connection: close` semantics; response framing with chunked encoding for streaming bodies; a streaming body type for uploads and SSE. No TLS, no HTTP/2, no HTTP/3, no compression in v1 (the proxy compresses).

Limits are part of the type, not configuration an app can forget: maximum request-line length, header count and total header bytes, body size, header read timeout, body read timeout, idle keep-alive timeout, concurrent connections. Every limit has a default that fails closed and a documented reason. Exceeding one yields 413, 431 or 408 and closes the connection.

The parser is a hand-written state machine over `&[u8]` with no allocation on the header path: header names and values are borrowed slices into the connection buffer until the request is handed to the router. It rejects bare CR, obsolete line folding, duplicate `Content-Length` with differing values, `Transfer-Encoding` combined with `Content-Length`, non-token method names, and whitespace before the colon. Those are the smuggling vectors; each has a regression test with the exact bytes.

The fun part: the parser is written to be provable. A property-based test asserts that for any byte sequence the parser either produces a request that re-serialises to a canonical form or rejects with a specific error, and never panics, never reads past the buffer, and never accepts two different framings for the same bytes. `cargo fuzz` targets exist for the request parser, the chunked decoder and the header value tokeniser, with the corpus committed. CPU-hours fuzzed are recorded in the crate README and gated in section 17.

Accept loop: one Tokio task per connection, a semaphore for the connection cap, graceful shutdown that stops accepting, lets in-flight requests finish within a drain window, and sends `Connection: close` on the last response of each connection. Unix socket support is first-class because the Omarchy deployment (section 16) puts Caddy and the app on the same box.

## 9. aor-router

Replaces axum and tower. A typed router, extractor set and fixed middleware stack; the layer where the framework's opinions live.

Routes are declared with a macro that checks the path template at compile time and produces a typed path-parameter struct:

```rust
route!(GET "/plugins/{slug}/versions/{version}" => plugins::show_version);
// generates: struct ShowVersionPath { slug: Slug, version: SemVer }
```

A handler is an `async fn` taking extractors in any order, as in axum, but the set is closed: `Path<T>`, `Query<T>`, `Form<T>`, `Json<T>`, `Session`, `Principal`, `RequestId`, `Body`. Applications cannot define new extractors in v1; that keeps the middleware ordering provable and is the kind of restriction one OS and one community permit. Extractors that fail produce the section 12 error shapes with stable codes.

Matching is a radix tree built at startup; conflicts (two routes that could match one path) are a startup error with both routes named. `routes --json` dumps the tree with method, path, handler, authentication requirement and, once `aor-verify` has run, the policy each route resolves to.

Middleware is a fixed stack in a fixed order: connection limits (in `aor-http`) → request ID → tracing span → security headers → session load → CSRF check for cookie-authenticated mutating methods → route match → extractors → handler → error mapping → response headers. Applications can add middleware only at two named slots (`before_route` and `after_handler`) and cannot reorder the rest. The ordering is tested by a suite that sends a CSRF-less POST with a valid session and asserts rejection before the handler runs.

Errors are a single `AppError` enum with a `code()` that is stable across versions and a `status()` mapping. `500` never carries internals; the request ID is the only diagnostic.

JSON routes live under `/api/v1`. Content negotiation never changes authentication behaviour: an API route with a cookie and no CSRF token is rejected, not silently treated as a browser request.

## 10. aor-db and aor-migrate

Replaces SeaORM, sqlx and refinery. The headline feature of the whole framework: SQL in application code is checked at compile time against the committed migrations, with no database running during the build.

`aor-migrate` owns ordered SQL migration files under `migrations/`, applies them under an advisory lock, records checksums, and marks non-transactional migrations explicitly. Nothing new there. What is new is that it also ships a build-time schema deriver: a `build.rs` step parses every committed migration's DDL (CREATE, ALTER, DROP for tables, columns, indexes, constraints and enums) and emits a `schema.rs` describing the final schema as Rust types. The DDL parser handles the PostgreSQL subset the framework generates plus a documented hand-written subset; unsupported statements are a build error naming the file and line, never silently skipped.

`aor-db` then provides a `sql!` macro that parses the query at compile time, resolves every table and column reference against `schema.rs`, infers parameter and result types, and generates a typed function. A query that names a column the migrations never created does not compile. Adding a column in a migration and forgetting to update a `SELECT *` view type is caught the same way, because `SELECT *` is rejected: columns are named.

```rust
let rows = sql!(
    "SELECT id, slug, name, updated_at FROM plugins WHERE owner_id = $1 ORDER BY updated_at DESC LIMIT $2",
    scope.owner_id(), limit
).fetch_all(&tx).await?;
// rows: Vec<{ id: Uuid, slug: Slug, name: String, updated_at: DateTime<Utc> }>
```

The SQL parser is a hand-written recursive-descent parser for the PostgreSQL dialect subset that covers SELECT with joins, CTEs, aggregates and window functions, INSERT ... RETURNING, UPDATE, DELETE, and the constructs the queue and session stores need (`FOR UPDATE SKIP LOCKED`, `ON CONFLICT`). It does not attempt the whole grammar; anything outside the subset is a compile error with a pointer to the raw-query escape hatch, `sql_unchecked!`, which `aor-verify` reports at every call site.

At runtime, `aor-db` is a thin pool over `tokio-postgres`: connections, prepared-statement caching, typed parameter encoding, and the `Tx` handle type that `aor-tx` (section 13) enforces. No ActiveRecord, no lazy relations, no query builder with a fluent API; relations are explicit queries. A development-mode query counter reports N+1 patterns per request.

Data conventions from earlier versions stand: UUID primary keys, UTC timestamps, explicit nullability, database constraints as the authority, integer minor units plus currency for money, optimistic locking on versioned rows, opt-in soft delete with explicit scope.

Embedded local mode uses SQLite through the same `sql!` macro until `aor-store` exists; the compile-time checker knows both dialects' subsets and rejects PostgreSQL-only constructs when the target is local.

## 11. aor-tmpl

Replaces Askama and MiniJinja. A template language compiled to Rust by a proc macro in release builds and interpreted from disk in development, with typed contexts in both modes and Omarchy theme awareness built in.

The language is small and deliberately Jinja-shaped so agents already know it: `{{ expr }}` with auto-escaping, `{% if %}`, `{% for %}`, `{% match %}` over Rust enums, `{% extends %}` and `{% block %}`, `{% include %}`, and a fixed set of filters. Raw HTML requires `{{ value | trusted }}` and the value must be of type `TrustedHtml`, constructed only through a named sanitiser; a `String` passed to `trusted` is a compile error.

Each template declares its context type:

```jinja
{# context: PluginShowView #}
<h1>{{ plugin.name }}</h1>
{% for v in versions %}<li>{{ v.semver }} — {{ v.published_at | date(tz) }}</li>{% endfor %}
```

In release, the proc macro reads the template at compile time, type-checks every expression against `PluginShowView`'s fields, and emits Rust that writes to a buffer. A typo in a field name is a compile error pointing at the template line. In development, the same parser runs at request time from the file on disk, and type mismatches surface as a rendered error page on the next request rather than a rebuild — this is what makes template editing cost under a second. Both modes share one parser and one type checker; the release mode is the interpreter with the interpretation done early.

Theme awareness: the framework ships a base stylesheet built entirely on CSS custom properties. At startup, or on a theme-change signal, `aor-tmpl` reads the active Omarchy theme (section 16), maps its palette onto those properties, and serves a `/_aor/theme.css` that the base layout includes. A generated page therefore matches the terminal, the bar and the launcher on the same desktop. For hosted apps with no local theme, the default palette is Tokyo Night. A per-user theme cookie and a theme picker component are in a later level.

Form helpers render inputs bound to the same input types the JSON handlers use, carry the CSRF token, preserve non-secret values on validation failure, and never repopulate password fields. Date and time rendering always states the timezone.

Assets: CSS and JS under `assets/` are content-hashed at build time and served with long cache headers by a built-in static handler; no Node in the default path. Templates and assets embed into the binary in release so an app is one file plus its database.

## 12. aor-session and auth

Replaces tower-sessions and Loco's auth. Small, fully specified, and the layer where the negative test matrix earns its place.

Sessions are server-side rows in the application database (PostgreSQL or the local store) keyed by an opaque 256-bit token from `getrandom`, stored hashed. The cookie carries the token only. Production cookies are `Secure`, `HttpOnly`, `SameSite=Lax` by default with `Strict` available per app, and are scoped to the configured public origin; an app that starts in production with no public origin configured refuses to start. Session rotation happens at login, logout and any privilege change; logout deletes the row; account-wide revocation deletes all rows for a principal. Idle and absolute expiries are both enforced server-side.

CSRF uses a per-session secret and a double-submit token rendered by the form helper and checked by the fixed middleware slot for every cookie-authenticated `POST`, `PUT`, `PATCH` and `DELETE`. `GET` never mutates; a handler mounted on `GET` that takes a mutating service signature is a `verify` rule violation.

Password auth: Argon2id via RustCrypto with parameters calibrated on the reference machine and recorded; registration (when enabled), login, logout, email verification, password reset with hashed single-use expiring tokens, and responses that do not reveal whether an account exists. Timing of the login path is equalised between unknown-user and wrong-password.

API tokens for the installer and the games: opaque, scoped, hashed at rest, revocable, with an expiry. The Arcade uses a per-game signing key so score submissions are HMAC-signed payloads rather than bearer tokens in a binary anyone can read; the key rotates and the server accepts the previous key for a grace window.

OIDC and third-party login are excluded from v1. Organisation tenancy is excluded. Both would need their own matrix rows before they could be trusted.

Everything above appears in the matrix as tests, not documentation: CSRF-less mutation rejected before the handler, token replay after logout rejected, reset token second use rejected, session fixation attempt rotated away, cookie without `Secure` refused at production startup.

## 13. aor-policy, aor-tx and aor-verify

Carried forward from v0.2 unchanged in intent, now sitting on the author's own runtime rather than Loco's. Repeated here because it is what makes the from-scratch stack safe to expose, and because every other crate is designed around these types.

| Component | What it is | What it prevents |
| --- | --- | --- |
| `Authorized<T, A>` | A newtype only a policy function can construct, carrying the principal and the permitted query scope; required by every `aor-db` function that reads or writes an owned resource | Fetch-by-ID without an ownership check; client-supplied parent IDs used as authority |
| `Tx` handle | Repository, enqueue and session functions take `&Tx`; only services call `begin()`; detached variants exist under `_unsafe` names | Effects committed independently of the domain change |
| `View<T>` | Public responses are explicit view types; entity types cannot derive the wire serialiser (a lint in `aor-verify`) | Password hashes and internal columns leaking |
| Negative matrix | Per authenticated resource: anonymous, other user's record, other user's parent, ownership reassignment, stale version, duplicate job delivery, CSRF-less mutation | The agent's green suite that tested only what it built |
| `verify` | Static: every route resolves to a handler whose service takes an `Authorized` scope; every `sql_unchecked!` and `_unsafe` call site listed; entity types with wire derives listed; `GET` routes with mutating signatures listed. Dynamic: matrix coverage and `tests/boundaries/` hash status. Output: human and versioned JSON with stable rule IDs | Having to read handlers to know whether the boundaries held |

Denial returns 404 for concealed resources, framework-wide; 403 only for resources declared public-existence. Ownership derives from the principal; an input type carrying an owner field does not compile against a scaffolded service signature.

The illustrative service is the same shape as v0.2, now on `aor-db`:

```rust
pub async fn publish_version(
    ctx: &AppContext,
    scope: Authorized<Plugin, Publish>,
    input: PublishVersion,
) -> Result<VersionView, AppError> {
    input.validate()?;
    let tx = ctx.db.begin().await?;
    let v = versions::insert_in_scope(&tx, &scope, input).await?;
    jobs::enqueue_in(&tx, ValidateManifestV1 { version_id: v.id }).await?;
    tx.commit().await?;
    Ok(VersionView::from(v))
}
```

`verify` gains one from-scratch-specific job: it reads `routes --json` and the middleware stack declaration and asserts the fixed ordering in section 9 has not been altered by a framework change. The framework's own test suite runs `verify` against the three reference apps on every commit.

## 14. aor-jobs

Replaces Underway and apalis. A durable, typed, versioned queue in the application database, designed around two guarantees: enqueue shares the domain transaction, and a dead worker can never acknowledge a job another worker has since claimed.

The job row: id, type name (a stable string, decoupled from the Rust type name so renames do not orphan queued work), payload version, payload bytes, queue, state, attempt, max attempts, run-at, lease-expires-at, lease generation, created/updated timestamps, redacted last-error. Claim is one statement: `UPDATE ... SET state='running', lease_generation = lease_generation + 1, ... WHERE id = (SELECT id ... FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING *`. Completion and failure are conditional on the claimed generation still matching; a stale worker's acknowledgement affects zero rows and is logged.

Delivery is at least once. Every handler declares its idempotency strategy in the type — `Idempotent::ByKey(fn)`, `Idempotent::Natural`, or `Idempotent::AtLeastOnceTolerated` with a doc-comment — and `verify` refuses a handler with none. Retries back off exponentially with jitter to a ceiling; exhausted jobs enter `failed` with inspection and explicit requeue via the CLI. Workers renew leases on a heartbeat while running; shutdown stops claiming, drains within a window, and lets unfinished leases expire.

Payload versioning is enforced: a handler registers the versions it accepts; a queued payload with an unknown version parks the job as `incompatible` rather than failing it, so a rolling deploy that has not yet shipped the new worker does not burn retries.

Scheduling arrives in a later level: stable schedule identifiers, unique occurrence keys so multiple schedulers do not double-enqueue, UTC by default, documented daylight-saving and missed-run policies.

Local mode runs the worker in-process against the local store with the same semantics; the in-memory test transport is selectable only under `cfg(test)`.

The reference job is `ValidateManifestV1` for the Plugin Registry: parse the uploaded plugin manifest, check it against the schema, record findings, and on success flip the version to published. It is deliberately the first job because a duplicate delivery must produce one published version, which the matrix tests.

## 15. aor-cli, aor-dev and the agent interface

Replaces the Loco CLI and watchexec. `clap` stays for argument parsing; everything the commands do is owned.

| Command | Contract |
| --- | --- |
| `new <name> [--local]` | Starter with the verification core, `AGENTS.md`, `tests/boundaries/`, `.aor/` ancestry, committed lockfile, pacman `PKGBUILD` skeleton, systemd unit templates. `--local` selects the embedded store and single-process mode |
| `scaffold resource <Name> [--parent <P>] [--public]` | Policy stub, service signatures, input and view types, migration, negative matrix, typed route declarations. No handler bodies, no templates; the agent writes those |
| `scaffold job <Name>` | Typed versioned payload, handler stub with a required idempotency declaration, duplicate-delivery test |
| `verify [--json]` | Section 13; frozen JSON schema per minor version, stable rule IDs |
| `doctor` | Prerequisites, PostgreSQL connectivity or local store health, configuration problems; read-only |
| `dev` | Owned inotify watcher over `src/`, `templates/`, `assets/`, `migrations/`; rebuild and restart on Rust changes with the last-good process labelled stale on a failed build; live template reload with no rebuild; asset re-hash; browser reload via a dev-only SSE endpoint |
| `routes [--json]` | Method, path, handler, auth requirement, resolved policy |
| `db migrate / status / reset` | Ordered migrations under a lock; `reset` is development-only and confirms |
| `jobs list / inspect / retry / park` | Queue operations |
| `pkg` | Build the release binary with embedded templates and assets, write the `PKGBUILD` and systemd units, produce a pacman package (section 16) |
| `upgrade --check` | Three-way diff against `.aor/` template ancestry; reports, never rewrites |

All commands: `--help`, non-interactive, `--json` where output is structured, stable diagnostic codes, nonzero exit on failure. Scaffolds are deterministic given version and inputs, refuse conflicts, and support `--dry-run` whose output equals the applied diff. Names and paths are validated; no user input reaches a shell string.

`aor-dev` is written on raw `inotify` via the `nix` crate rather than `notify`, because owning the watcher is part of the challenge and because Omarchy is Linux only. It debounces, coalesces renames, handles editor swap-file patterns, and survives the watched directory being recreated.

The agent interface is v0.2's: `AGENTS.md` under 400 lines as a reference card (boundary rules as prohibitions with the correct alternative, directory table, exact commands with expected exit codes, what never to edit, how to read a `verify` failure); the CLI as the deterministic contract; the `verify` report as the thing pasted into a pull request. No prompts, no model-specific instructions, no network calls. An MCP wrapper over `verify --json` and `routes --json` is a post-1.0 option.

## 16. Omarchy integration

This is where one OS and one community turn into features rather than constraints.

**Theme.** Omarchy keeps the active theme under `~/.config/omarchy/current/theme/` as a set of per-application colour files. `aor-tmpl` reads that directory at startup and on change (watched by the same inotify layer as `aor-dev`), maps the palette onto the base stylesheet's custom properties, and serves `/_aor/theme.css`. The mapping is a small table per known theme file format; unknown themes fall back to Tokyo Night. Hosted apps have no local theme; they take a configured default and, in a later level, a per-user picker listing Omarchy's shipped themes so a registry page can match the visitor's desktop.

**Packaging.** `aor pkg` produces a pacman package: the release binary with embedded templates and assets, a `PKGBUILD`, an `install` script that creates a system user, and systemd units for `app-serve.service`, `app-work.service` and `app-migrate.service` (a oneshot the serve and work units `Requires=`). Services run as the dedicated user, with `ProtectSystem=strict`, `PrivateTmp`, `NoNewPrivileges` and a read-write path only for the configured data directory. Configuration lives in `/etc/<app>/config.toml` with secrets in a separate root-only file loaded through systemd `LoadCredential`.

**Local mode.** `new --local` produces an app with no PostgreSQL: the embedded store (SQLite until `aor-store` exists), serve and work in one process, sessions in the store, listening on a Unix socket under `$XDG_RUNTIME_DIR` or a loopback port. A `.desktop` entry and a Walker launcher entry open the app in the default browser. This is how a task manager or a theme picker becomes an installable Omarchy app with one binary and no daemon.

**Deployment shape for hosted apps.** One Arch box, Caddy terminating TLS and speaking HTTP/1.1 over a Unix socket to `app-serve`, PostgreSQL local, `app-work` alongside. Caddy's config is generated by `aor pkg --caddy` from the app's public origin. Deployment is `pacman -U`, then `systemctl restart`; the migrate oneshot runs first under the advisory lock.

**Installer and games as API clients.** The Plugin Registry exposes `/api/v1/plugins` for a future `omarchy-plugin install` command; the Arcade exposes `/api/v1/scores` with HMAC-signed submissions from the games. Both API surfaces are declared types with generated OpenAPI, and drift fails CI.

**Out of scope here.** Native Omarchy windows, shell plugins, Waybar modules and Hyprland integration are separate projects that consume these HTTP APIs. AoR does not render GTK, does not talk to the compositor, and does not become a desktop toolkit.

## 17. Test strategy, fuzzing and release gates

Three test programmes, because three kinds of thing can be wrong.

**Parsers are fuzzed.** `aor-http` (request parser, chunked decoder, header tokeniser), `aor-db` (SQL parser, DDL parser), `aor-tmpl` (template parser) and `aor-migrate` (checksum and file ordering) each have `cargo fuzz` targets with committed corpora. A crate's README records CPU-hours fuzzed and the last crash-free date. Property tests assert the parser invariants in section 8 for every parser: no panic, no out-of-bounds read, reject-or-canonicalise, and one framing per input.

**Semantics are tested against real backends.** PostgreSQL and the local store both run in CI; the matrix and the queue tests run against both. One database per test worker; no shared rollback fixtures where a worker holds its own connection. Clocks and external providers are injected.

**Boundaries are tested by the matrix.** Section 13's negative tests per resource, emitted by scaffold, hash-tracked by `verify`.

| Gate | Required evidence |
| --- | --- |
| Parser safety | Each fuzz target has run ≥ 200 CPU-hours crash-free on the current parser revision before the crate is used by a public app; regression bytes for every historical crash committed as tests |
| Smuggling resistance | The request-smuggling corpus (CL.TE, TE.CL, TE.TE, obs-fold, bare CR, duplicate CL) is rejected byte-for-byte with the documented error |
| Compile-time schema | A migration adding, renaming and dropping a column each produce the expected compile error in a downstream test crate; `sql_unchecked!` sites are all listed by `verify` |
| Ownership and reassignment | As v0.2: other user's record and parent → 404; anonymous → 401; owner or parent reassignment rejected |
| Transaction integrity | Forced failure after enqueue rolls back row and job; no mail before commit |
| Duplicate delivery and recovery | Each job type run twice yields one effect; worker death, lease expiry, stale acknowledgement and retry exhaustion behave as specified |
| Stale writes and concurrent uniqueness | 409 on stale version; two simultaneous creates yield one row and one 409 |
| Session security | CSRF-less mutation rejected before handler; rotation at login; revocation at logout; reset token single-use; production refuses insecure cookie configuration |
| Middleware ordering | The fixed order in section 9 is asserted by tests that would fail if any slot moved |
| Information protection | No hash, internal flag or unlisted column in any response, log or OpenAPI schema |
| Theme | Every shipped Omarchy theme maps without a missing property; unknown theme falls back cleanly |
| Packaging | `aor pkg` output installs on a clean Arch VM, the units start, the migrate oneshot runs first, the service is confined as specified |
| Local mode | `new --local` app runs with no daemon, survives a kill mid-job, resumes the job |
| Boundary file integrity and generator safety | As v0.2 |
| Documentation | Every command in `AGENTS.md` and every README example is exercised in CI |

Security review: before the Plugin Registry accepts uploads from anyone but the author, `aor-http`, `aor-session` and the CSRF middleware are read end to end by at least one person who did not write them, and the findings are fixed or recorded. Passing the gates is necessary, not sufficient.

## 18. Targets

Provisional until Level 0 records the reference machine (an Omarchy install, its CPU and memory, Rust version, PostgreSQL version). Medians and p95, never a best run.

| Measure | Initial target |
| --- | --- |
| `aor-http` plaintext hello-world throughput on loopback, single core | Within 25 % of hyper on the same machine; measured, not assumed |
| `aor-http` request parse | Under 1 µs median for a typical browser request line plus headers, zero allocations |
| Fuzz coverage | ≥ 200 CPU-hours per parser target before public use; ≥ 1,000 before 1.0 |
| Compile-time `sql!` cost | Under 50 ms added build time per 100 queries on the reference machine |
| Warm incremental controller edit to running | Median under 5 s, p95 under 10 s, with mold and the dev profile set by `new` |
| Template edit to refresh | Under 1 s in development |
| Full rebuild of the Plugin Registry, warm cache | Under 3 min |
| Idle `app-serve` RSS | Under 60 MiB for the registry; the from-scratch stack should beat the v0.2 figure |
| Warm startup to ready | Under 1 s with migrations current |
| Queue claim latency | Under 5 ms median from enqueue commit to handler start, single worker, idle system |
| Boundary defects per agent-built resource, with core | 0 reaching `verify` pass |
| Agent sessions to green on a scaffolded resource | Median ≤ 2 across three current coding agents |
| Theme switch to page update | Under 500 ms from Omarchy theme change to refreshed `/_aor/theme.css` |
| Local-mode binary | Under 15 MiB stripped, including embedded templates and assets |

If a target proves unrealistic, publish the measurement and revise it. Never remove a limit or a boundary check to hit a number.

## 19. Delivery: levels

Each level ends with something running on Omarchy and a piece the author wanted to build. Levels are sequential; a level's exit condition is a running app, not a finished library.

1. **Level 0 — The socket.** `aor-http` on Tokio: parser, limits, keep-alive, chunked bodies, Unix sockets, graceful shutdown. Fuzz targets and the smuggling corpus from day one. Exit: a static file server for the Fixing Everything archive runs behind Caddy on the reference box and has 200 crash-free fuzz hours. The piece to build: the zero-allocation parser and its invariant proofs.
2. **Level 1 — Routes and pages.** `aor-router` with the `route!` macro, closed extractor set, fixed middleware stack; `aor-tmpl` in interpreted mode with typed contexts and the Omarchy theme reader; `aor-dev` on inotify. Exit: the Fixing Everything site renders from templates, live-reloads on edit, and wears the local theme. The piece: the template type checker.
3. **Level 2 — The schema at compile time.** `aor-migrate` with the DDL deriver; `aor-db` with the `sql!` macro, pool and `Tx`; the local SQLite target. Exit: Fixing Everything stores editions in PostgreSQL through checked queries; a deliberately wrong column fails the build. The piece: the SQL parser and the schema derivation.
4. **Level 3 — Accounts and boundaries.** `aor-session` (sessions, CSRF, password auth), `aor-policy`, `aor-tx`, the scaffold, the matrix, `aor-verify`. Exit: the Plugin Registry with accounts and owner-scoped plugins passes the full matrix when its second resource is built by an agent from a two-sentence brief. The piece: `Authorized<T, A>` ergonomics good enough that agents reach for it unprompted.
5. **Level 4 — Durable work.** `aor-jobs` with transactional enqueue, lease fencing, idempotency declarations, payload versioning; `ValidateManifestV1`; production mail through jobs. Exit: the registry accepts uploads, validates asynchronously, survives a worker kill mid-job with one published version. The piece: the fencing proof under fault injection.
6. **Level 5 — Ship it to Arch.** `aor pkg`: release binary with embedded assets, `PKGBUILD`, hardened systemd units, Caddy config, migrate oneshot. `new --local` with `.desktop` and Walker entries. Exit: the registry installs from a pacman package on a clean Omarchy VM; a local-mode task manager installs and runs with no daemon. The piece: the packaging pipeline and the confinement profile.
7. **Level 6 — Public.** Security review of `aor-http`, `aor-session` and CSRF by someone else; 1,000 fuzz hours per parser; the Arcade leaderboard with signed submissions; OpenAPI drift checks; targets measured and published. Exit: the registry is open to Omarchy plugin authors. The piece: the Arcade's HMAC scheme and rate limiting.
8. **Level 7 — Local storage.** `aor-store`: a single-file append-only log with a B-tree index, MVCC snapshots for readers, WAL-style crash recovery, and the `sql!` subset targeted at it, replacing SQLite in local mode. Exit: the local task manager runs on `aor-store` and survives `kill -9` during a write with no corruption across 10,000 randomised crash tests. The piece: all of it.
9. **Later levels, by demonstrated need.** SSE for the Arcade's live board; scheduled jobs; a per-user theme picker for hosted apps; WebSockets after a further fuzz programme; `upgrade` with three-way merge; an MCP wrapper over `verify` and `routes`.

The order puts the dangerous parser first so it accumulates fuzz time under every later level, puts the compile-time schema before accounts so the session and policy stores are built on checked SQL, and puts the embedded store last because nothing public depends on it.

## 20. Risks

| Risk | Likelihood | Mitigation |
| --- | --- | --- |
| A parser bug in `aor-http` reaches a public app | Medium | Fuzz-hour gates before public use; reverse proxy in front always; the smuggling corpus as regression tests; the honesty clause's replacement rule |
| The DDL and SQL parsers cover too little of PostgreSQL and every real query hits `sql_unchecked!` | Medium | Grow the subset from the three reference apps' actual queries; `verify` counts unchecked sites so the ratio is visible; the subset is a published list |
| Compile times grow with the proc macros | Medium | Cache derived schema per migration set; `sql!` macro output is small; measure per-100-queries cost from Level 2 onward |
| `Authorized<T, A>` is too rigid and agents route around it | Medium | `verify` lists every escape hatch; the matrix tests outcomes, not types; the sessions-to-green target catches bad ergonomics |
| The closed extractor and middleware sets block a real need | Medium | Two named extension slots; a new extractor is a framework release, not an app hack; the need is recorded before the slot is opened |
| Lease fencing or transactional enqueue has a race the tests miss | Low with fault injection | Deterministic fault-injection harness over the queue; property tests over interleavings; the Level 4 exit requires a kill mid-job |
| The theme mapping breaks when Omarchy changes its theme layout | High over time | The mapping is one table in one file; unknown layout falls back to the default palette and logs once |
| A layer stops being fun and becomes maintenance | Certain for at least one | The named replacement crate goes back in; the spec is amended; nothing else depends on the layer's internals because each crate is independently usable |
| The registry attracts adversarial uploads | Certain once public | Manifest validation as a sandboxed job; size and type limits; uploads private until validated; malware scanning as a documented integration point; no execution of uploaded content on the server |
| Agents edit or delete matrix tests to pass | High without mitigation | Hash tracking in `verify`; `AGENTS.md` prohibition; loud in the report |
| The retired expansion of AoR resurfaces in public material | Low | Acronym only; CI grep on the expansion |

## 21. Definition of done and evidence

v0.3 is realised when:

- the Omarchy Plugin Registry runs in public on `aor-http`, `aor-router`, `aor-db`, `aor-tmpl`, `aor-session` and `aor-jobs`, packaged by `aor pkg`, behind Caddy, with no hyper, axum, SeaORM, sqlx, Askama, tower-sessions or queue crate anywhere in its dependency tree;
- a coding agent given a two-sentence brief adds an owner-scoped resource to it in at most two sessions, `verify` passes, and the developer merges from the policy file, the service signatures and the report;
- a deliberately wrong column name, a detached enqueue, an entity on the wire and a CSRF-less mutation are each caught by the compiler, `verify` or the matrix before merge;
- every parser has its recorded fuzz hours and a crash-free date on the current revision;
- a local-mode Omarchy app installs from a pacman package, launches from Walker, wears the active theme and runs with no daemon;
- each owned crate's README opens with the crate it replaces and one paragraph on what is done differently.

**Evidence and hypotheses.** Ecosystem facts checked 30 September 2026: [Loco](https://github.com/loco-rs/loco) at 1.0.1 is the framework v0.2 would have extended and the closest existing comparison; [hyper](https://docs.rs/hyper), [axum](https://docs.rs/axum), [sqlx](https://docs.rs/sqlx), [Askama](https://docs.rs/askama) and [tower-sessions](https://docs.rs/tower-sessions) are the crates each owned layer replaces and the baselines for section 18's comparative targets. The Omarchy theme directory layout is taken from the current Omarchy repository and is the fact most likely to move; the mapping is isolated accordingly.

Hypotheses to be settled by the levels: that a hand-written HTTP/1.1 parser can be brought to a defensible state with the stated fuzz programme; that a PostgreSQL DDL and SQL subset small enough to parse at compile time is large enough for three real apps; that the typed template checker's development-mode interpretation and release-mode compilation can share one implementation without drift; that `Authorized<T, A>` is ergonomic enough for agents unprompted.

**Naming.** AoR, acronym only; the expansion is retired and must not appear in public material. Check crate and domain availability before the first public commit.

**Unresolved before Level 0:** reference machine; PostgreSQL support window; the exact HTTP/1.1 subset (which optional features are rejected outright versus supported); whether `aor-http` exposes a streaming body trait or a concrete type; who performs the Level 6 security review.
