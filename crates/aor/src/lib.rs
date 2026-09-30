//! Experimental AoR foundation. See docs/status.md for the unimplemented specification.
pub use aor_http as http;
pub use aor_router as router;
pub use aor_tmpl as tmpl;
pub mod prelude {
    pub use aor_http::{Request, Response};
    pub use aor_router::{
        AppError, Body, Context, Form, Json, Path, Query, RequestId, Router, SemVer, Slug, handler,
        route, route_path,
    };
    pub use aor_tmpl::{Template, TemplateContext};
}

pub use aor_db as db;
pub use aor_migrate as migrate;

pub use aor_policy as policy;
pub use aor_session as session;
pub use aor_tx as tx;
