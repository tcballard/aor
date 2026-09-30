//! Owned radix routing and public-request middleware. Authenticated routes are gated
//! until the session and policy layers exist; there is no pretend authentication mode.
extern crate self as aor_router;
use aor_http::{Request, Response};
pub use aor_macros::{handler, route};
mod extract;
pub use extract::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

pub const MIDDLEWARE: &[&str] = &[
    "transport_limits",
    "request_id",
    "tracing",
    "security_headers",
    "reject_unsupported_credentials",
    "before_route",
    "route_match",
    "extractors",
    "handler",
    "after_handler",
    "error_mapping",
    "response_headers",
];
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppError {
    NotFound,
    MethodNotAllowed,
    BadPath,
    BadQuery,
    BadJson,
    BadForm,
    UnsupportedMediaType,
    BodyTooLarge,
    AuthenticationUnavailable,
    Internal,
}
impl AppError {
    pub fn code(self) -> &'static str {
        match self {
            Self::NotFound => "NOT_FOUND",
            Self::MethodNotAllowed => "METHOD_NOT_ALLOWED",
            Self::BadPath => "INVALID_PATH",
            Self::BadQuery => "INVALID_QUERY",
            Self::BadJson => "INVALID_JSON",
            Self::BadForm => "INVALID_FORM",
            Self::UnsupportedMediaType => "UNSUPPORTED_MEDIA_TYPE",
            Self::BodyTooLarge => "BODY_TOO_LARGE",
            Self::AuthenticationUnavailable => "AUTH_NOT_IMPLEMENTED",
            Self::Internal => "INTERNAL_ERROR",
        }
    }
    pub fn status(self) -> u16 {
        match self {
            Self::NotFound => 404,
            Self::MethodNotAllowed => 405,
            Self::BadPath | Self::BadQuery | Self::BadJson | Self::BadForm => 400,
            Self::UnsupportedMediaType => 415,
            Self::BodyTooLarge => 413,
            Self::AuthenticationUnavailable => 503,
            Self::Internal => 500,
        }
    }
    pub fn response(self, id: &RequestId) -> Response {
        Response::new(
            self.status(),
            serde_json::to_vec(
                &serde_json::json!({"error":{"code":self.code(),"request_id":id.0}}),
            )
            .unwrap(),
        )
        .header("Content-Type", "application/json")
        .unwrap()
    }
}
#[derive(Clone, Debug)]
pub struct RequestId(pub String);
#[derive(Debug)]
pub struct Params {
    parameters: BTreeMap<String, String>,
}
impl Params {
    pub fn get<T: std::str::FromStr>(&self, name: &str) -> Result<T, AppError> {
        self.parameters
            .get(name)
            .ok_or(AppError::BadPath)?
            .parse()
            .map_err(|_| AppError::BadPath)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct Slug(String);
impl std::str::FromStr for Slug {
    type Err = AppError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty()
            || s.len() > 100
            || !s
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || s.starts_with('-')
            || s.ends_with('-')
        {
            Err(AppError::BadPath)
        } else {
            Ok(Self(s.to_owned()))
        }
    }
}
impl TryFrom<String> for Slug {
    type Error = AppError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}
impl std::fmt::Display for Slug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub struct Context {
    pub request: Request,
    pub path: Params,
    pub request_id: RequestId,
}
impl Context {
    pub async fn json<T: serde::de::DeserializeOwned>(self) -> Result<T, AppError> {
        if self
            .request
            .headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case("content-type"))
            .count()
            != 1
        {
            return Err(AppError::UnsupportedMediaType);
        }
        let media = self
            .request
            .header("content-type")
            .and_then(|v| std::str::from_utf8(v).ok())
            .unwrap_or("");
        if !media
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("application/json")
        {
            return Err(AppError::UnsupportedMediaType);
        }
        let bytes = self
            .request
            .body
            .collect()
            .await
            .map_err(|_| AppError::BodyTooLarge)?;
        serde_json::from_slice(&bytes).map_err(|_| AppError::BadJson)
    }
    pub fn query(&self) -> Result<BTreeMap<String, String>, AppError> {
        query(self.request.target.split_once('?').map_or("", |(_, q)| q))
    }
}
pub fn query(q: &str) -> Result<BTreeMap<String, String>, AppError> {
    let mut out = BTreeMap::new();
    if q.is_empty() {
        return Ok(out);
    }
    for item in q.split('&') {
        if out.len() >= 128 {
            return Err(AppError::BadQuery);
        }
        let (k, v) = item.split_once('=').unwrap_or((item, ""));
        let k = decode(k, true)?;
        let v = decode(v, true)?;
        if k.is_empty() || out.insert(k, v).is_some() {
            return Err(AppError::BadQuery);
        }
    }
    Ok(out)
}
fn decode(s: &str, form: bool) -> Result<String, AppError> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                if i + 2 >= bytes.len() {
                    return Err(AppError::BadPath);
                }
                let h = (bytes[i + 1] as char)
                    .to_digit(16)
                    .ok_or(AppError::BadPath)?;
                let l = (bytes[i + 2] as char)
                    .to_digit(16)
                    .ok_or(AppError::BadPath)?;
                let b = (h * 16 + l) as u8;
                if b == 0 || (!form && (b == b'/' || b == b'\\')) {
                    return Err(AppError::BadPath);
                }
                out.push(b);
                i += 3;
            }
            b'+' if form => {
                out.push(b' ');
                i += 1;
            }
            b => {
                if b < 32 || b == 127 || b == b'\\' {
                    return Err(AppError::BadPath);
                }
                out.push(b);
                i += 1;
            }
        }
    }
    let s = String::from_utf8(out).map_err(|_| AppError::BadPath)?;
    if s.chars().any(char::is_control) {
        return Err(AppError::BadPath);
    }
    Ok(s)
}
type Handler = Arc<
    dyn Fn(Context) -> Pin<Box<dyn Future<Output = Result<Response, AppError>> + Send>>
        + Send
        + Sync,
>;
#[derive(Clone, Debug, Serialize)]
pub struct RouteInfo {
    pub method: String,
    pub path: String,
    pub handler: String,
    pub authentication: &'static str,
    pub policy: Option<String>,
}
pub struct Route {
    pub info: RouteInfo,
    handler: Handler,
}
impl Route {
    pub fn public<F, Fut>(method: &str, path: &str, name: &str, handler: F) -> Self
    where
        F: Fn(Context) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Response, AppError>> + Send + 'static,
    {
        Self {
            info: RouteInfo {
                method: method.to_owned(),
                path: path.to_owned(),
                handler: name.to_owned(),
                authentication: "public",
                policy: None,
            },
            handler: Arc::new(move |ctx| Box::pin(handler(ctx))),
        }
    }
}
#[derive(Default)]
struct Node {
    static_children: BTreeMap<String, Node>,
    parameter: Option<Box<Node>>,
    endpoints: BTreeMap<String, usize>,
}
#[derive(Debug)]
pub struct RequestTrace {
    pub request_id: RequestId,
    pub method: String,
    pub route: Option<String>,
    pub status: u16,
    pub elapsed: std::time::Duration,
}
type BeforeRoute = Arc<dyn Fn(&Request, &RequestId) -> Result<(), AppError> + Send + Sync>;
type AfterHandler = Arc<dyn Fn(&RequestTrace, Response) -> Response + Send + Sync>;
pub struct Router {
    root: Node,
    routes: Vec<Route>,
    ids: AtomicU64,
    before_route: Option<BeforeRoute>,
    after_handler: Option<AfterHandler>,
    tracing: bool,
}
fn segments(path: &str) -> Result<Vec<&str>, String> {
    if !path.starts_with('/')
        || path.contains(['?', '#', '%', '\\'])
        || path.contains("//")
        || (path.len() > 1 && path.ends_with('/'))
        || !path.is_ascii()
        || path.bytes().any(|b| b < 33 || b == 127)
    {
        return Err(format!("invalid route: {path}"));
    }
    if path == "/" {
        Ok(vec![])
    } else {
        Ok(path[1..].split('/').collect())
    }
}
fn param(s: &str) -> bool {
    s.starts_with('{') && s.ends_with('}')
}
impl Router {
    pub fn new(routes: Vec<Route>) -> Result<Self, String> {
        let mut root = Node::default();
        for (i, r) in routes.iter().enumerate() {
            if !["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"]
                .contains(&r.info.method.as_str())
            {
                return Err(format!("invalid method: {}", r.info.method));
            }
            let parts = segments(&r.info.path)?;
            let mut seen = std::collections::BTreeSet::new();
            for part in &parts {
                if part.contains(['{', '}'])
                    && (!param(part)
                        || part.len() < 3
                        || !part[1..part.len() - 1]
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                        || part.as_bytes()[1].is_ascii_digit()
                        || !seen.insert(*part))
                {
                    return Err(format!("invalid parameter in {}", r.info.path));
                }
            }
            for other in &routes[..i] {
                let same_method = r.info.method == other.info.method;
                let old = segments(&other.info.path)?;
                if same_method
                    && old.len() == parts.len()
                    && old
                        .iter()
                        .zip(&parts)
                        .all(|(a, b)| a == b || param(a) || param(b))
                {
                    return Err(format!(
                        "route conflict: {} {} ({}) and {} {} ({})",
                        r.info.method,
                        r.info.path,
                        r.info.handler,
                        other.info.method,
                        other.info.path,
                        other.info.handler
                    ));
                }
            }
            let mut node = &mut root;
            for part in parts {
                node = if param(part) {
                    node.parameter.get_or_insert_with(Default::default)
                } else {
                    node.static_children.entry(part.to_owned()).or_default()
                };
            }
            node.endpoints.insert(r.info.method.clone(), i);
        }
        Ok(Self {
            root,
            routes,
            ids: AtomicU64::new(1),
            before_route: None,
            after_handler: None,
            tracing: false,
        })
    }
    pub fn before_route(
        mut self,
        hook: impl Fn(&Request, &RequestId) -> Result<(), AppError> + Send + Sync + 'static,
    ) -> Self {
        self.before_route = Some(Arc::new(hook));
        self
    }
    /// Runs only after a successful handler; cannot convert a middleware/extractor denial.
    pub fn after_handler(
        mut self,
        hook: impl Fn(&RequestTrace, Response) -> Response + Send + Sync + 'static,
    ) -> Self {
        self.after_handler = Some(Arc::new(hook));
        self
    }
    pub fn tracing(mut self, enabled: bool) -> Self {
        self.tracing = enabled;
        self
    }
    pub fn routes(&self) -> Vec<&RouteInfo> {
        let mut routes: Vec<_> = self.routes.iter().map(|r| &r.info).collect();
        routes.sort_by(|a, b| (&a.path, &a.method).cmp(&(&b.path, &b.method)));
        routes
    }
    fn lookup(&self, path: &str, method: &str) -> Result<(usize, Params), AppError> {
        let raw: Vec<_> = if path == "/" {
            vec![]
        } else {
            path.strip_prefix('/')
                .ok_or(AppError::BadPath)?
                .split('/')
                .collect()
        };
        let parts: Vec<String> = raw
            .into_iter()
            .map(|s| decode(s, false))
            .collect::<Result<_, _>>()?;
        if parts.iter().any(|s| s.is_empty() || s == "." || s == "..") {
            return Err(AppError::BadPath);
        }
        fn visit(n: &Node, parts: &[String], method: &str, found_path: &mut bool) -> Option<usize> {
            if parts.is_empty() {
                *found_path |= !n.endpoints.is_empty();
                return n
                    .endpoints
                    .get(method)
                    .or_else(|| {
                        if method == "HEAD" {
                            n.endpoints.get("GET")
                        } else {
                            None
                        }
                    })
                    .copied();
            }
            if let Some(child) = n.static_children.get(&parts[0]) {
                if let Some(i) = visit(child, &parts[1..], method, found_path) {
                    return Some(i);
                }
            }
            n.parameter
                .as_ref()
                .and_then(|child| visit(child, &parts[1..], method, found_path))
        }
        let mut found = false;
        let i = visit(&self.root, &parts, method, &mut found).ok_or(if found {
            AppError::MethodNotAllowed
        } else {
            AppError::NotFound
        })?;
        let names = segments(&self.routes[i].info.path).map_err(|_| AppError::Internal)?;
        let parameters = names
            .iter()
            .zip(parts)
            .filter(|(n, _)| param(n))
            .map(|(n, v)| (n[1..n.len() - 1].to_owned(), v))
            .collect();
        Ok((i, Params { parameters }))
    }
    pub async fn handle(&self, request: Request) -> Response {
        let started = std::time::Instant::now();
        let method = request.method.clone();
        let mut route_name = None;
        let id = RequestId(format!(
            "aor-{:016x}",
            self.ids.fetch_add(1, Ordering::Relaxed)
        ));
        // No cookie/API credential is treated as authenticated before Level 3 exists.
        let result =
            if request.header("cookie").is_some() || request.header("authorization").is_some() {
                Err(AppError::AuthenticationUnavailable)
            } else if let Some(error) = self
                .before_route
                .as_ref()
                .and_then(|hook| hook(&request, &id).err())
            {
                Err(error)
            } else {
                let path = request.target.split('?').next().unwrap_or("/");
                match self.lookup(path, &request.method) {
                    Ok((i, path)) => {
                        route_name = Some(self.routes[i].info.path.clone());
                        (self.routes[i].handler)(Context {
                            request,
                            path,
                            request_id: id.clone(),
                        })
                        .await
                    }
                    Err(e) => Err(e),
                }
            };
        let mut trace = RequestTrace {
            request_id: id.clone(),
            method,
            route: route_name,
            status: result.as_ref().map_or_else(|e| e.status(), |r| r.status),
            elapsed: started.elapsed(),
        };
        let response = match result {
            Ok(response) => {
                if let Some(hook) = &self.after_handler {
                    hook(&trace, response)
                } else {
                    response
                }
            }
            Err(e) => e.response(&id),
        };
        let response = Self::response_headers(response, &id).unwrap_or_else(|_| {
            Self::response_headers(AppError::Internal.response(&id), &id)
                .expect("fixed error headers fit transport limits")
        });
        trace.status = response.status;
        trace.elapsed = started.elapsed();
        if self.tracing {
            eprintln!(
                "{}",
                serde_json::json!({"request_id":id.0,"method":trace.method,"route":trace.route,"status":trace.status,"elapsed_us":trace.elapsed.as_micros()})
            );
        }
        response
    }
    fn response_headers(response: Response, id: &RequestId) -> Result<Response, std::io::Error> {
        response.set_header("X-Request-Id",&id.0)?.set_header("X-Content-Type-Options","nosniff")?
            .set_header("Referrer-Policy","no-referrer")?
            .set_header("Content-Security-Policy","default-src 'none'; style-src 'self'; img-src 'self'; script-src 'self'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'")
    }
}
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for AppError {}

#[cfg(test)]
mod tests {
    use super::*;
    async fn handler(_: Context) -> Result<Response, AppError> {
        Ok(Response::new(200, vec![]))
    }
    #[test]
    fn conflict_names_both_routes() {
        let err = Router::new(vec![
            route!(GET "/plugins/new"=>handler),
            route!(GET "/plugins/{slug}"=>handler),
        ])
        .err()
        .unwrap();
        assert!(err.contains("/plugins/new") && err.contains("/plugins/{slug}"));
    }
    #[test]
    fn method_specific_radix_backtracking() {
        let r = Router::new(vec![
            route!(POST "/plugins/new"=>handler),
            route!(GET "/plugins/{slug}"=>handler),
        ])
        .unwrap();
        let (_, p) = r.lookup("/plugins/new", "GET").unwrap();
        assert_eq!(p.get::<Slug>("slug").unwrap().to_string(), "new");
        assert!(matches!(
            r.lookup("/plugins/new", "DELETE"),
            Err(AppError::MethodNotAllowed)
        ));
    }
    #[test]
    fn encoded_separators_and_traversal_rejected() {
        let r = Router::new(vec![route!(GET "/plugins/{slug}"=>handler)]).unwrap();
        for p in [
            "/plugins/%2fetc",
            "/plugins/%2e%2e",
            "/plugins/%00",
            "/plugins/%5cetc",
            "/plugins/%ff",
        ] {
            assert!(r.lookup(p, "GET").is_err());
        }
    }
    #[test]
    fn duplicate_query_and_malformed_escape_rejected() {
        assert!(query("x=1&x=2").is_err());
        assert!(query("x=%zz").is_err());
        assert_eq!(query("x=hello+world").unwrap()["x"], "hello world");
    }
}
