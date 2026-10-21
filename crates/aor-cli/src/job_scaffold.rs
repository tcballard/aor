use std::{
    collections::BTreeMap,
    io::{self, Write},
    path::{Path, PathBuf},
};
fn valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name != "Self"
        && name.as_bytes()[0].is_ascii_uppercase()
        && name.bytes().all(|b| b.is_ascii_alphanumeric())
}
fn module(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if i > 0 && c.is_ascii_uppercase() {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}
fn safe_path(root: &Path, relative: &Path) -> io::Result<()> {
    let mut path = root.to_path_buf();
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(io::Error::other("AOR_SCAFFOLD_PATH"));
        }
        path.push(component);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(io::Error::other("AOR_SCAFFOLD_SYMLINK"));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
            _ => {}
        }
    }
    Ok(())
}
pub fn plan(root: &Path, name: &str) -> io::Result<BTreeMap<PathBuf, String>> {
    if !valid(name) {
        return Err(io::Error::other(
            "AOR_JOB_NAME: expected a PascalCase identifier",
        ));
    }
    if !root.join("Cargo.toml").is_file() || !root.join("src").is_dir() {
        return Err(io::Error::other(
            "AOR_JOB_ROOT: expected an application crate",
        ));
    }
    let module = format!("{}_job", module(name));
    let payload = format!("{name}V1");
    let source = format!(
        r#"use aor_db::{{Pool, Uuid}};
use aor_jobs::{{Claim, Idempotent, Job}};
use serde::{{Deserialize, Serialize}};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct {payload} {{ pub id: Uuid }}
impl Job for {payload} {{
    const NAME: &'static str = "app.{module}";
    const VERSION: i64 = 1;
    const IDEMPOTENCY: Idempotent = choose_idempotency_strategy();
    fn key(&self) -> String {{ self.id.to_string() }}
}}
pub async fn handle(pool: &Pool, claim: &Claim) -> aor_jobs::Result<()> {{
    implement_fenced_handler(pool, claim).await
}}
compile_error!("AOR_JOB_INCOMPLETE: declare idempotency and implement fenced effects before registering this job");
"#
    );
    let test = format!(
        r#"#[path = "../src/{module}.rs"]
mod job;
#[test]
fn duplicate_delivery_has_one_effect() {{
    compile_error!("AOR_JOB_MATRIX_INCOMPLETE: assert duplicate effects, rollback, worker death, stale claims and unsupported payload versions on SQLite and PostgreSQL");
}}
"#
    );
    let files = BTreeMap::from([
        (PathBuf::from(format!("src/{module}.rs")), source),
        (PathBuf::from(format!("tests/{module}_duplicate.rs")), test),
    ]);
    for path in files.keys() {
        safe_path(root, path)?;
        if root.join(path).exists() {
            return Err(io::Error::other(format!(
                "AOR_JOB_CONFLICT: {}",
                path.display()
            )));
        }
    }
    Ok(files)
}
pub fn run(root: &Path, name: &str, dry_run: bool) -> io::Result<()> {
    let files = plan(root, name)?;
    if !dry_run {
        for (path, contents) in &files {
            safe_path(root, path)?;
            let target = root.join(path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(target)?;
            file.write_all(contents.as_bytes())?;
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"schema_version":1,"dry_run":dry_run,"files":files,"next":"Add the module to lib.rs, declare idempotency, implement fenced effects and outcome tests, register the stable name/version, and review the boundary inventory before running verify."})
        )?
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deterministic_conflicts_and_symlinks_fail_closed() {
        let root = std::env::temp_dir().join(format!("aor-job-scaffold-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname=\"fixture\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();
        let first = plan(&root, "NotifyOwner").unwrap();
        assert_eq!(first, plan(&root, "NotifyOwner").unwrap());
        assert!(first.values().all(|s| s.contains("compile_error!")));
        for name in ["../Escape", "Self", "a", "Bad;Touch", ""] {
            assert!(plan(&root, name).is_err());
        }
        run(&root, "NotifyOwner", true).unwrap();
        assert!(!root.join("tests").exists());
        run(&root, "NotifyOwner", false).unwrap();
        for (path, contents) in first {
            assert_eq!(std::fs::read_to_string(root.join(path)).unwrap(), contents);
        }
        assert!(plan(&root, "NotifyOwner").is_err());
        std::fs::remove_dir_all(root.join("tests")).unwrap();
        std::os::unix::fs::symlink(root.join("src"), root.join("tests")).unwrap();
        assert!(plan(&root, "OtherJob").is_err());
        assert!(!root.join("src/other_job_job.rs").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
