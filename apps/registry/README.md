# Plugin Registry — L3 development fixture

Accounts, private owner-scoped plugins and child versions run on the owned HTTP,
router and checked SQL layers. This is a development API, not a public release.
Versions are drafts; manifest validation and publication jobs arrive at L4.

```sh
export AOR_SQLITE=/tmp/aor-registry.sqlite
export AOR_DEV_MAIL_SPOOL=/tmp/aor-registry-mail
cargo run -p aor-registry -- migrate
cargo run -p aor-registry -- serve
```

The server listens on `127.0.0.1:3001`. `AOR_SOCKET` selects a Unix socket.
`AOR_DATABASE_URL` selects PostgreSQL instead of SQLite. Run `migrate` explicitly;
startup never silently changes the schema. Do not mix this fixture's migration
history with another app. Auth migration 001 is embedded before plugin 002 and
version 003; never call the auth-only migration runner on this combined history.

POST JSON to `/accounts/register` with `email` and a password of 12–1024 bytes.
Every account POST needs `Origin: http://localhost:3001`. Registration returns the
same 202 response for new and existing accounts. The **development-only** mail
spool is a private directory (0700), with one private JSON delivery file (0600).
Read its verification token locally and POST `{"token":"..."}` to
`/accounts/verify`, then POST credentials to `/accounts/login`. Verification and
reset tokens never appear in HTTP responses or logs. Production rejects this
spool and disables registration until a production delivery adapter is supplied.

Keep both login cookies. Cookie-authenticated mutations additionally require an
`X-CSRF-Token` header matching `aor_dev_csrf`. Production uses `__Host-` cookie names,
Secure, HttpOnly for the session, Path=/ and SameSite=Lax. The generic router also
accepts `_csrf` in URL-encoded forms; `aor_tmpl::csrf_field` constructs the hidden
field after validating its token alphabet. Registry endpoints themselves accept JSON.

| Endpoint | JSON input / behavior |
| --- | --- |
| GET /plugins, GET /versions | Only this account's rows |
| POST /plugins | `{"name":"My plugin"}` |
| POST /versions | `{"name":"1.0.0","plugin_id":"UUID"}`; stored parent must belong to caller |
| GET /plugins/{id}, GET /versions/{id} | Own row or concealed 404 |
| PATCH either resource/{id} | `{"name":"Updated","version":1}`; stale version returns 409 |
| DELETE either resource/{id} | `{"version":1}`; delete child versions before their parent |
| POST /accounts/logout | Delete this session, expire cookies |
| POST /accounts/revoke-all | Revoke all account sessions/API tokens, expire cookies |
| POST /accounts/request-reset | `{"email":"..."}`; generic 202 and private delivery |
| POST /accounts/reset | `{"token":"...","password":"..."}`; single use and revoke old credentials |
| POST /accounts/tokens | `{"scopes":["plugins:read"],"lifetime_seconds":3600}`; show opaque token once |
| POST /accounts/revoke-token | `{"token":"..."}`; session-only, scoped to this account |

Bearer tokens use `Authorization: Bearer ...`; scopes are exact resource/action
pairs, expire within 90 days and cannot manage accounts. Mixing bearer credentials
and a session cookie is rejected. Inputs reject unknown fields, including ownership
or parent reassignment. Outputs contain explicit projections, never auth entities.

`AOR_ENV=production` requires `AOR_PUBLIC_ORIGIN=https://...`. Current origin parsing
accepts DNS names and IPv4 with an optional nonzero port; IPv6 literals are not
supported. Origin configuration is exact, with no trailing slash or implicit aliases.
Auth's portable schema represents canonical UUIDs as TEXT and UTC seconds as BIGINT;
this intentionally differs from PostgreSQL-native UUID/timestamptz conventions.

## Validation

`cargo test -p aor-registry --test versions_matrix` exercises both resources through
the real HTTP connection over an in-memory duplex stream and the SQLite database.
The ignored PostgreSQL case is mandatory in database CI with an isolated database.
The root plugin has no parent; the other-parent row applies to Version. Duplicate
job delivery is not applicable until the L4 job exists, and is not counted as a pass.
The account and router suites cover authentication, CSRF and token/session replay.
