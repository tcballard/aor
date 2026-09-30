# Build the AoR HTTP foundation and development archive

AoR starts from an empty repository. The v0.3 specification needs a working, bounded HTTP server before the database and security layers can be built on it.

This change implements the owned HTTP parser/server, a running archive fixture, initial routing and interpreted templates, Omarchy colour adapters, inotify reload, the last-good development process and foundation evidence tooling. It also adds parser fuzz targets, denial tests, CI, and a source-bound fuzz evidence workflow.

Validation: 28 local tests passed; three downstream compile-failure cases passed; the live archive and failed-rebuild recovery checks passed; formatting and warnings-as-errors Clippy passed. Four parser targets completed ASan-only smoke fuzzing. Unix socket creation and LeakSanitizer are restricted in the local sandbox; CI retains both full checks.

Scope: this is the initial development foundation. It does not complete Level 0's reference-host/fuzz gate or Level 1's full API. Database/schema compilation, authorization, sessions, CSRF, durable jobs, packaging, the remaining reference applications and the local store are not implemented. `cargo aor verify --json` correctly fails public readiness. See `docs/status.md` for the exact gaps.

