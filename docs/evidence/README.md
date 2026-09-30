# Development evidence — 30 September 2026

This is evidence for the initial foundation, not a v0.3 acceptance report.

- `cargo test --locked --workspace -- --skip unix_socket_roundtrip`: **28 passed**, 1 filtered. The excluded Unix socket test fails locally with EPERM during socket creation. CI runs it without a skip; remote CI has not run because publication is blocked.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: passed.
- `cargo fmt --all --check`: passed.
- `scripts/check-compile-fail.py`: three downstream crates failed for the expected diagnostics (duplicate route parameter, noncanonical route, unsupported context field type).
- `scripts/smoke.py`: real TCP process; templates, CSS, health, encoded traversal rejection, credentials denied before handlers, inotify/SSE reload and template-error display passed.
- `scripts/dev-smoke.py`: real CLI/process; intentional compile error retains last-good server and displays stale status; restored source rebuilds and restarts successfully. Its log deliberately contains the compiler error used by the test.
- `verify-development.json`: foundation checks pass. `verify-public.json`: public readiness fails as required. JSON version 1.
- `routes.json`: actual archive route registry. `doctor.json`: local Linux toolchain diagnostics.
- Instrumented fuzzing: request, chunked, tokens and template completed ASan smoke runs with leak detection disabled because LeakSanitizer cannot inspect processes in this sandbox. Those runs have `release_eligible: false` and contribute **zero** public-release hours. Initial failed attempts are retained as failed records. No sanitizer restrictions in the host were changed.
- Successful HTTP smoke counts: request 1,069,028 inputs; chunked 617,225; tokens 2,503,679. A follow-up template smoke with a complete summary executed 81,792 inputs. These are smoke tests, not safety proofs.
- Current corpus is coverage-minimised with named regression seeds preserved. The corpus files are committed; full campaign reruns must retain/upload their resulting corpus and logs.

Local environment: Linux 6.18.44 x86_64, Rust 1.88.0. This is not the specified Omarchy reference machine. Caddy/PostgreSQL and clean Arch installation were not verified here. No throughput, latency or binary-size targets are claimed. No independent security review is claimed.

The acceptance gates in the original specification remain unchanged.
