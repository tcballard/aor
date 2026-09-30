//! Experimental AoR foundation. See docs/status.md for the unimplemented specification.
pub use aor_http as http;
pub use aor_router as router;
pub use aor_tmpl as tmpl;
pub mod prelude {
    pub use aor_http::{Request, Response};
    pub use aor_router::{AppError, Context, Router, Slug, route};
    pub use aor_tmpl::{Template, TemplateContext};
}
