//! Development evidence checker. This is not yet the route-to-policy verifier.
mod boundaries;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io, path::Path};
use syn::{spanned::Spanned, visit::Visit};
#[derive(Serialize, Debug)]
pub struct Finding {
    pub rule_id: String,
    pub status: &'static str,
    pub detail: String,
    pub public_gate: bool,
}
#[derive(Serialize, Debug)]
pub struct Report {
    pub schema_version: u32,
    pub scope: &'static str,
    pub development_pass: bool,
    pub public_ready: bool,
    pub findings: Vec<Finding>,
}
#[derive(Serialize, Debug)]
pub struct Callsite {
    pub path: String,
    pub line: usize,
    pub kind: &'static str,
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn source_hash(root: &Path, relative: &str) -> io::Result<String> {
    let mut files = Vec::new();
    collect(&root.join(relative), &mut files)?;
    files.sort();
    let mut h = Sha256::new();
    for path in files {
        if path.extension().is_some_and(|e| e == "rs") {
            h.update(
                path.strip_prefix(root)
                    .map_err(io::Error::other)?
                    .to_string_lossy()
                    .as_bytes(),
            );
            h.update([0]);
            h.update(std::fs::read(path)?);
            h.update([0]);
        }
    }
    Ok(format!("{:x}", h.finalize()))
}
fn collect(path: &Path, out: &mut Vec<std::path::PathBuf>) -> io::Result<()> {
    for e in std::fs::read_dir(path)? {
        let e = e?;
        let kind = e.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            collect(&e.path(), out)?;
        } else if kind.is_file() {
            out.push(e.path());
        }
    }
    Ok(())
}
pub fn inspect_source(path: &str, source: &str) -> Result<Vec<Callsite>, syn::Error> {
    struct Scan<'a> {
        path: &'a str,
        out: Vec<Callsite>,
    }
    impl<'ast> Visit<'ast> for Scan<'_> {
        fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
            if let syn::Expr::Path(p) = &*node.func {
                if p.path
                    .segments
                    .last()
                    .is_some_and(|s| s.ident.to_string().ends_with("_unsafe"))
                {
                    self.out.push(Callsite {
                        path: self.path.to_owned(),
                        line: node.span().start().line,
                        kind: "unsafe_escape",
                    });
                }
            }
            syn::visit::visit_expr_call(self, node);
        }
        fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
            if node.method.to_string().ends_with("_unsafe") {
                self.out.push(Callsite {
                    path: self.path.to_owned(),
                    line: node.span().start().line,
                    kind: "unsafe_escape",
                });
            }
            syn::visit::visit_expr_method_call(self, node);
        }
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            if node
                .path
                .segments
                .last()
                .is_some_and(|s| s.ident == "sql_unchecked")
            {
                self.out.push(Callsite {
                    path: self.path.to_owned(),
                    line: node.span().start().line,
                    kind: "unchecked_sql",
                });
            }
            syn::visit::visit_macro(self, node);
        }
    }
    let file = syn::parse_file(source)?;
    let mut scan = Scan { path, out: vec![] };
    scan.visit_file(&file);
    Ok(scan.out)
}
#[derive(Deserialize)]
struct Evidence {
    runs: Vec<Run>,
}
#[derive(Deserialize)]
struct Run {
    target: String,
    source_sha256: String,
    cpu_seconds: f64,
    crash_free: bool,
    #[serde(default)]
    release_eligible: bool,
}
pub fn verify(root: &Path) -> io::Result<Report> {
    let mut findings = Vec::new();
    let mut add = |rule: &str, ok: bool, detail: String, public_gate: bool| {
        findings.push(Finding {
            rule_id: rule.to_owned(),
            status: if ok { "pass" } else { "fail" },
            detail,
            public_gate,
        })
    };
    add(
        "AOR-DEV-001",
        root.join("Cargo.lock").is_file(),
        "Workspace lockfile is required".into(),
        false,
    );
    let mut sources = Vec::new();
    collect(&root.join("crates"), &mut sources)?;
    collect(&root.join("apps"), &mut sources)?;
    sources.sort();
    for path in sources
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
    {
        let relative = path
            .strip_prefix(root)
            .map_err(io::Error::other)?
            .to_string_lossy()
            .into_owned();
        match inspect_source(&relative, &std::fs::read_to_string(path)?) {
            Ok(calls) => {
                for c in calls {
                    add(
                        "AOR-ESCAPE-001",
                        false,
                        format!("{}:{} {}", c.path, c.line, c.kind),
                        false,
                    );
                }
            }
            Err(e) => add("AOR-PARSE-001", false, format!("{relative}: {e}"), false),
        }
    }
    // Registered owned-resource modules are audited with the supported direct-call shape.
    let mut resources = Vec::new();
    for path in &sources {
        if path.extension().is_some_and(|e| e == "rs") {
            let relative = path
                .strip_prefix(root)
                .map_err(io::Error::other)?
                .to_string_lossy();
            let source = std::fs::read_to_string(path)?;
            let owned = relative.starts_with("apps/")
                && relative != "apps/registry/src/accounts.rs"
                && boundaries::is_resource(&source).map_err(io::Error::other)?;
            if owned {
                resources.push(relative.to_string());
            }
            for f in boundaries::inspect(&relative, &source, owned).map_err(io::Error::other)? {
                add(&f.rule_id, false, f.detail, false);
            }
        }
    }
    add(
        "AOR-POLICY-002",
        resources.len() >= 2,
        format!(
            "Resolved owned-resource modules: {}. The Registry requires its two resource fixtures.",
            resources.join(", ")
        ),
        false,
    );
    let middleware = std::fs::read_to_string(root.join("crates/aor-router/src/lib.rs"))?;
    add("AOR-MIDDLEWARE-001",boundaries::middleware_is_fixed(&middleware).map_err(io::Error::other)?,"Authenticated middleware declaration must retain the fixed session/CSRF ordering exercised by router denial tests".into(),false);
    let auth_migration = std::fs::read(root.join("crates/aor-session/migrations/001_auth.sql"))?;
    add(
        "AOR-AUTH-001",
        auth_migration == std::fs::read(root.join("apps/registry/migrations/001_auth.sql"))?,
        "Registry auth migration must match the checked session schema".into(),
        false,
    );
    let integrity = root.join(".aor/boundaries.json");
    let manifest: BTreeMap<String, String> = std::fs::read(&integrity)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let mut actual = Vec::new();
    for base in [
        "crates/aor-http/tests",
        "crates/aor-router/tests",
        "crates/aor-sql/tests",
        "crates/aor-db/tests",
        "crates/aor-session/tests",
        "apps/registry/tests",
        "tests/boundaries",
    ] {
        if root.join(base).is_dir() {
            collect(&root.join(base), &mut actual)?;
        }
    }
    actual.sort();
    let actual: BTreeMap<String, String> = actual
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .map(|p| {
            Ok((
                p.strip_prefix(root)
                    .map_err(io::Error::other)?
                    .to_string_lossy()
                    .into_owned(),
                hash(&std::fs::read(p)?),
            ))
        })
        .collect::<io::Result<_>>()?;
    add("AOR-BOUNDARY-001",!manifest.is_empty()&&manifest==actual,"Tracked boundary tests must match .aor/boundaries.json exactly; changes require human review".into(),false);
    let lock = std::fs::read_to_string(root.join("Cargo.lock"))?;
    let forbidden = [
        "hyper",
        "axum",
        "tower",
        "sea-orm",
        "sqlx",
        "askama",
        "minijinja",
        "tower-sessions",
        "apalis",
        "underway",
        "notify",
    ];
    let found: Vec<_> = forbidden
        .into_iter()
        .filter(|name| {
            lock.lines()
                .any(|line| line == format!("name = \"{name}\""))
        })
        .collect();
    add(
        "AOR-LAYERS-001",
        found.is_empty(),
        format!("Displaced crates present: {}", found.join(", ")),
        false,
    );
    let evidence: Evidence = std::fs::read(root.join("docs/evidence/fuzz.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Evidence { runs: vec![] });
    for (target, source) in [
        ("request", "crates/aor-http/src"),
        ("chunked", "crates/aor-http/src"),
        ("tokens", "crates/aor-http/src"),
        ("template", "crates/aor-tmpl/src"),
        ("sql", "crates/aor-sql/src"),
    ] {
        let revision = source_hash(root, source)?;
        let hours = evidence
            .runs
            .iter()
            .filter(|r| {
                r.target == target
                    && r.source_sha256 == revision
                    && r.crash_free
                    && r.release_eligible
                    && r.cpu_seconds.is_finite()
                    && r.cpu_seconds >= 0.0
            })
            .map(|r| r.cpu_seconds / 3600.0)
            .fold(0.0, |total, hours| total + hours);
        add(
            "AOR-FUZZ-001",
            hours >= 200.0,
            format!("{target}: {hours:.6} / 200 CPU-hours on current source {revision}"),
            true,
        );
    }
    let missing: Vec<_> = [
        "aor-db",
        "aor-migrate",
        "aor-policy",
        "aor-tx",
        "aor-session",
        "aor-jobs",
        "aor-testkit",
    ]
    .into_iter()
    .filter(|c| !root.join("crates").join(c).join("Cargo.toml").exists())
    .collect();
    add(
        "AOR-SPEC-001",
        false,
        format!(
            "v0.3 incomplete. Unimplemented crates: {}. General indirect route dispatch, durable jobs, packaging, reference-machine calibration and public release review remain incomplete.",
            missing.join(", ")
        ),
        true,
    );
    add(
        "AOR-REVIEW-001",
        false,
        "Independent HTTP/session/CSRF review not recorded; no public-use clearance".into(),
        true,
    );
    let development_pass = findings.iter().all(|f| f.public_gate || f.status == "pass");
    let public_ready = findings.iter().all(|f| f.status == "pass");
    Ok(Report {
        schema_version: 1,
        scope: "development-foundation",
        development_pass,
        public_ready,
        findings,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scanner_uses_ast_not_comments() {
        let src = "// sql_unchecked! and foo_unsafe() are documentation\nfn f() { sql_unchecked!(\"SELECT x\"); db.enqueue_unsafe(); }";
        let found = inspect_source("x.rs", src).unwrap();
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|x| x.line == 2));
    }
}
