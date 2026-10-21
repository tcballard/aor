use std::{
    collections::BTreeMap,
    io::{self, Write},
    path::{Path, PathBuf},
};
fn valid(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.as_bytes()[0].is_ascii_uppercase()
        && s.bytes().all(|b| b.is_ascii_alphanumeric())
}
fn snake(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && c.is_ascii_uppercase() {
            out.push('_')
        }
        out.push(c.to_ascii_lowercase())
    }
    out
}
pub fn plan(
    root: &Path,
    name: &str,
    parent: Option<&str>,
    public: bool,
) -> io::Result<BTreeMap<PathBuf, String>> {
    if !valid(name) || parent.is_some_and(|p| !valid(p) || p == name) {
        return Err(io::Error::other(
            "resource and parent must be distinct PascalCase identifiers",
        ));
    }
    if !root.join("Cargo.toml").is_file()
        || !root.join("src").is_dir()
        || !root.join("migrations").is_dir()
    {
        return Err(io::Error::other(
            "--root must be an existing AoR application crate",
        ));
    }
    let module = format!("{}s", snake(name));
    let parent_field = parent.map(|p| format!("{}_id", snake(p)));
    let parent_table = parent.map(|p| format!("{}s", snake(p)));
    let mut files = BTreeMap::new();
    let source = format!(
        r#"//! Generated boundary contract. Implement services and route bodies, then remove the compile error.
use aor_db::{{Uuid, Tx}};
use aor_policy::{{Authorized, Resource, Read, Create, Update, Delete}};
use aor_router::AppError;
use serde::{{Deserialize, Serialize}};
aor_router::route_path!(pub {name}Path {{id:Uuid}});
pub enum {name} {{}}
impl Resource for {name} {{const NAME: &'static str = "{module}"; const PUBLIC_EXISTENCE: bool = {public};}}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {{pub name:String,{parent_input}}}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {{pub name:String,pub version:i64}}
#[derive(Serialize)]
pub struct Output {{pub id:Uuid,pub name:String,pub version:i64,{parent_input}}}
// Repository functions require (&mut Tx, &Authorized<{name}, Action>).
// Services own the transaction and use aor_tx::begin to revalidate the scope.
// Parent IDs are checked against the principal's stored ownership in that transaction.
pub trait Services {{
 fn list(&self,scope:Authorized<{name},Read>)->impl std::future::Future<Output=Result<Vec<Output>,AppError>>+Send;
 fn get(&self,scope:Authorized<{name},Read>,id:Uuid)->impl std::future::Future<Output=Result<Output,AppError>>+Send;
 fn create(&self,scope:Authorized<{name},Create>,input:Input)->impl std::future::Future<Output=Result<Output,AppError>>+Send;
 fn update(&self,scope:Authorized<{name},Update>,id:Uuid,input:Change)->impl std::future::Future<Output=Result<Output,AppError>>+Send;
 fn delete(&self,scope:Authorized<{name},Delete>,id:Uuid,version:i64)->impl std::future::Future<Output=Result<(),AppError>>+Send;
}}
pub const ROUTES:&[(&str,&str,&str)]=&[("GET","/{module}","Read"),("POST","/{module}","Create"),("GET","/{module}/{{id}}","Read"),("PATCH","/{module}/{{id}}","Update"),("DELETE","/{module}/{{id}}","Delete")];
compile_error!("AOR_SCAFFOLD_INCOMPLETE: implement the policy/service/repository route boundary and negative tests");
"#,
        parent_input = parent_field
            .as_ref()
            .map_or(String::new(), |p| format!("pub {p}:Uuid,"))
    );
    files.insert(PathBuf::from(format!("src/{module}.rs")), source);
    let mut next = 1u64;
    for entry in std::fs::read_dir(root.join("migrations"))? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if let Some((prefix, _)) = name.split_once('_') {
            if let Ok(n) = prefix.parse::<u64>() {
                next = next.max(
                    n.checked_add(1)
                        .ok_or_else(|| io::Error::other("migration sequence overflow"))?,
                );
            }
        }
    }
    let parent_sql = match (&parent_field, &parent_table) {
        (Some(f), Some(t)) => format!(" {f} TEXT NOT NULL REFERENCES {t}(id),\n"),
        _ => String::new(),
    };
    files.insert(PathBuf::from(format!("migrations/{next:03}_{module}.sql")),format!("CREATE TABLE {module} (\n id TEXT NOT NULL PRIMARY KEY,\n owner_id TEXT NOT NULL REFERENCES aor_users(id),\n{parent_sql} name TEXT NOT NULL,\n version BIGINT NOT NULL DEFAULT 1\n);\nCREATE INDEX {module}_owner ON {module}(owner_id);\n"));
    files.insert(PathBuf::from(format!("tests/{module}_matrix.rs")),format!(r#"//! Required outcomes for {name}: anonymous; other owner; {parent_case}; ownership
//! reassignment; stale update/delete; missing CSRF before handler; scoped token denial.
//! Exercise the actual router on SQLite and the isolated PostgreSQL CI database.
//! No durable job exists at L3; duplicate job delivery must be added with L4 jobs.
compile_error!("AOR_MATRIX_INCOMPLETE: implement outcome assertions, never replace with passing labels");
"#,parent_case=if parent.is_some(){"other user's parent and nonexistent parent"}else{"root resource: parent case not applicable"}));
    for path in files.keys() {
        if root.join(path).exists() {
            return Err(io::Error::other(format!(
                "refusing to overwrite {}",
                path.display()
            )));
        }
    }
    Ok(files)
}
pub fn run(
    root: &Path,
    name: &str,
    parent: Option<&str>,
    public: bool,
    dry_run: bool,
) -> io::Result<()> {
    let files = plan(root, name, parent, public)?;
    if !dry_run {
        for (path, contents) in &files {
            let target = root.join(path);
            if let Some(dir) = target.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)?;
            file.write_all(contents.as_bytes())?;
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"schema_version":1,"dry_run":dry_run,"files":files.keys().map(|p|p.to_string_lossy()).collect::<Vec<_>>(),"next":"Add the module to lib.rs, implement services and routes, implement the denial matrix, review migrations, run cargo test and cargo aor verify. Scaffolds intentionally fail compilation until completed."})
        )?
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_path_injection() {
        for n in ["../Bad", "Bad/Name", "Bad;touch", "lower", "", "Évil"] {
            assert!(!valid(n));
        }
        assert!(valid("Version"));
    }
}
