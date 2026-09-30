# AoR

A from-scratch Rust web framework for Omarchy. Built from the socket up, with typed boundaries and verification as its spine.

Implementation follows the product and technical specification v0.3. This repository is under active development; it is not ready for public application traffic.

## The honesty clause

1. Every layer here has a mature crate that does it better today. AoR exists because building it is the point.
2. The HTTP parser, session handling and CSRF implementation are the author's and have had the review described in the implementation evidence and no more. Deploy behind a reverse proxy; do not put it on the public internet without one.
3. If a security defect is found in a layer the author cannot fix within a week, that layer is replaced by the crate it displaced and the spec is amended. Pride is not a release gate.

AoR is a working acronym. No crate or domain has been reserved. Packages remain unpublished while implementation and release evidence are developed.

## Run the development archive

```bash
cargo run -p aor-archive -- serve
```

Open `http://127.0.0.1:3000`. It serves a development fixture, not Tom's actual published editions.

```bash
cargo aor dev
cargo aor routes --json
cargo aor verify --development --json
cargo test --locked --workspace
```

To use persistent editions locally:

```bash
cargo run -p aor-archive -- migrate --sqlite archive.sqlite
cargo run -p aor-archive -- import apps/archive/fixtures/editions.json --sqlite archive.sqlite
cargo run -p aor-archive -- serve --sqlite archive.sqlite
```

For PostgreSQL, set `AOR_DATABASE_URL` and omit `--sqlite`. The importer stores
editions through SQL checked against committed migrations at build time.
See [archive operations](apps/archive/README.md) and the [SQL subset](crates/aor-sql/README.md).

`cargo aor verify --json` currently exits **1**: the public-use fuzzing/review gates and the rest of v0.3 are unmet. `--development` checks only foundation integrity and is not deployment clearance.

[Implementation status](docs/status.md) · [Specification v0.3](docs/specification-v0.3.md) · [Contributor reference](AGENTS.md)
