# Public routing through Level 2

```rust,ignore
use aor_router::*;
route_path!(pub ShowVersionPath { slug: Slug, version: SemVer });
#[derive(serde::Deserialize)]
struct Options { limit: u32 }
#[handler]
async fn show(Query(options): Query<Options>, Path(path): Path<ShowVersionPath>,
              request_id: RequestId) -> Result<aor_http::Response, AppError> {
    Ok(aor_http::Response::new(200, format!("{}: {}", path.slug, options.limit)))
}
let route = route!(GET "/plugins/{slug}/{version}" => show);
```

Named path structs have explicit field types via `route_path!`; AoR does not infer
a business identifier's Rust type from its spelling. `#[handler]` accepts only
Path, Query, Form, Json, Session, Principal, RequestId and Body, in arbitrary order.
At most one body-consuming extractor is allowed. Query/Form support flat serde
structs, scalar numbers, booleans, enums and optional fields; duplicate keys and
malformed encodings fail. JSON uses serde and a single application/json header.
The underlying bounded HTTP body still controls allocation and deadlines.

`Context` remains available for framework adapters/closures with captured state;
its old untyped `Path` map is now called `Params`. Applications can implement
FromPath for domain types, but cannot implement the sealed extractor trait.

Middleware order is fixed. Credential rejection occurs before `before_route`;
that hook sees an immutable request and may reject it. `after_handler` runs only
on handler success, before mandatory response headers, and cannot turn a denied
extractor or credential into success. `.tracing(true)` emits one JSON request span
with request ID, method, route pattern, status and elapsed microseconds. It does
not log query strings, bodies, cookies, tokens or path parameter values.

Session and Principal always return AUTH_NOT_IMPLEMENTED. Cookies/bearer requests
are rejected before hooks/handlers. Real session loading and mutation CSRF belong
together with Level 3's auth/policy boundary; no placeholder authenticated state
is offered in Level 1/2. The registry's eventual protected middleware will fill
that position without allowing arbitrary middleware reordering.
