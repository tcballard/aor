# aor-http

Mature alternative: **hyper**. Owns the HTTP/1.1 parser, framing and Tokio serving loop. Request headers borrow the input buffer and the framing subset is intentionally strict.

Experimental. See [implementation status](../../docs/status.md) for limits and unmet release gates. Plain `cargo build` and `cargo test` work at the workspace root.

## Supported hop

HTTP/1.1 over loopback TCP or a pre-created Unix listener, behind a same-host reverse proxy. No TLS, HTTP/2, HTTP/3, compression or WebSockets. There is no public-bind opt-out.

Request line and headers use CRLF only. Exactly one valid Host is mandatory. Absolute-form, CONNECT, TRACE, upgrades, chunk extensions, trailers and non-chunked transfer coding are rejected. Duplicate Content-Length is always rejected (stricter than the identical-value allowance). TE+CL and duplicate TE are rejected and close the connection. Connection supports only close and keep-alive tokens.

Body bytes stream to a two-slot channel. Ignored bodies are drained and checked before a response is written. Applications must consume/validate input before effects; the transport cannot undo an application side effect. Streaming response producers are trusted application code and must bound their own queued chunks.

## Default bounds

| Limit | Default | Reason |
| --- | --- | --- |
| Request line | 8 KiB | Bounded URI/method parsing |
| Header section | 32 KiB | Bound cookies and proxy metadata |
| Headers | 96 | Fixed stack storage |
| Decoded body | 8 MiB | Bounded upload budget |
| Chunk size line | 1 KiB | Bound framing overhead |
| Chunks | 65,536 | Bound tiny-chunk work |
| Connections | 256 | Semaphore cap |
| Requests/connection | 100 | Bound connection reuse |
| Headers / body | 10 s / 30 s | Absolute read deadlines |
| Idle / write | 15 s / 15 s | Limit idle peers and slow response consumers |
| Handler / drain | 60 s / 30 s | Bound application and shutdown waits |

Zero limits and invalid header-count bounds are refused. Limits are always present on every server/connection.

## Evidence

Fuzz CPU-hours and crash-free completion dates are in `../../docs/evidence/fuzz.json`, keyed to the entire current HTTP source hash. The README makes no cumulative claim across revisions. No target has reached the required 200-hour public-app gate. No independent security review is recorded. TCP tests have run locally; the current sandbox denies Unix socket creation. CI retains the Unix test without a skip.

Protocol reference: [RFC 9112](https://www.rfc-editor.org/rfc/rfc9112.html). AoR's stricter subset is stated above.

Recorded qualifying CPU-hours at this checkpoint: request **0**, chunk decoder **0**, header tokens **0**. Local ASan smoke completed on 30 September 2026, with LeakSanitizer unavailable; those runs are explicitly ineligible for the release gate.
