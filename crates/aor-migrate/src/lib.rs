//! Ordered, checksum-locked migrations and an offline Rust schema generator.
pub use aor_sql::{Dialect, Schema};
use sha2::{Digest, Sha256};
use std::{fmt, path::Path};
#[derive(Debug)]
pub struct Error(pub String);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Debug)]
pub struct Migration {
    pub name: String,
    pub sql: String,
    pub checksum: String,
    pub transactional: bool,
}
pub fn load(path: impl AsRef<Path>, dialect: Dialect) -> Result<Vec<Migration>> {
    let mut schema = Schema::default();
    let mut out = Vec::new();
    for file in aor_sql::migration_files(path.as_ref()).map_err(Error)? {
        let sql = std::fs::read_to_string(&file).map_err(|e| Error(e.to_string()))?;
        schema
            .apply(&sql, dialect)
            .map_err(|e| Error(format!("{}:{e}", file.display())))?;
        let transactional = !sql
            .lines()
            .next()
            .is_some_and(|l| l.trim() == "-- aor: non-transactional");
        if !transactional && dialect == Dialect::Sqlite {
            return Err(Error("SQLite migrations must be transactional".into()));
        }
        out.push(Migration {
            name: file.file_name().unwrap().to_string_lossy().into_owned(),
            checksum: format!("{:x}", Sha256::digest(sql.as_bytes())),
            sql,
            transactional,
        });
    }
    Ok(out)
}
/// Call from build.rs. Directory and file tracking makes newly added migrations rebuild.
pub fn build(path: impl AsRef<Path>, dialect: Dialect) -> Result<Schema> {
    let path = path.as_ref();
    println!("cargo:rerun-if-changed={}", path.display());
    let schema = Schema::from_dir(path, dialect).map_err(Error)?;
    let mut code = String::from("// Generated from committed migrations. Do not edit.\n");
    for (name, table) in &schema.tables {
        let rust_name = pascal(name);
        code.push_str(&format!(
            "#[derive(Debug,Clone)] pub struct {rust_name} {{\n"
        ));
        for col in table.columns.values() {
            let ty = match col.ty {
                aor_sql::Type::Bool => "bool",
                aor_sql::Type::I32 => "i32",
                aor_sql::Type::I64 => "i64",
                aor_sql::Type::Text | aor_sql::Type::Enum(_) => "String",
                aor_sql::Type::Uuid => "::aor_db::Uuid",
                aor_sql::Type::Timestamp => "::aor_db::DateTime<::aor_db::Utc>",
                aor_sql::Type::Bytes => "Vec<u8>",
            };
            let ty = if col.nullable {
                format!("Option<{ty}>")
            } else {
                ty.into()
            };
            code.push_str(&format!("pub r#{}: {},\n", col.name, ty));
        }
        code.push_str("}\n");
    }
    let out = std::env::var("OUT_DIR").map_err(|e| Error(e.to_string()))?;
    std::fs::write(Path::new(&out).join("schema.rs"), code).map_err(|e| Error(e.to_string()))?;
    Ok(schema)
}
fn pascal(s: &str) -> String {
    s.split('_')
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect()
}
#[cfg(feature = "runtime")]
mod runtime {
    use super::*;
    use aor_db::{Executor, Lease, Pool, Value};
    impl From<aor_db::Error> for Error {
        fn from(e: aor_db::Error) -> Self {
            Self(e.to_string())
        }
    }
    const LOCK: i64 = 0x414f525f4d494752;
    pub async fn apply(pool: &Pool, migrations: &[Migration]) -> Result<usize> {
        let mut lease = pool.acquire().await?;
        let dialect = pool.dialect();
        lease.session_dirty(true);
        if dialect == Dialect::Postgres {
            lease
                .batch(&format!("SELECT pg_advisory_lock({LOCK})"))
                .await?;
        } else {
            lease.batch("BEGIN IMMEDIATE").await?;
        }
        let result = apply_locked(&mut lease, migrations).await;
        if result.is_ok() {
            if dialect == Dialect::Postgres {
                lease
                    .batch(&format!("SELECT pg_advisory_unlock({LOCK})"))
                    .await?;
            } else {
                lease.batch("COMMIT").await?;
            }
            lease.session_dirty(false);
        }
        // On any error/cancellation, connection disposal releases locks and rolls back.
        result
    }
    async fn apply_locked(lease: &mut Lease, migrations: &[Migration]) -> Result<usize> {
        let d = lease.dialect();
        lease.batch("CREATE TABLE IF NOT EXISTS _aor_migrations (name TEXT PRIMARY KEY, checksum TEXT NOT NULL, dirty BOOLEAN NOT NULL)").await?;
        let history = lease
            .query(
                d,
                "SELECT name, checksum, dirty FROM _aor_migrations ORDER BY name",
                &[],
            )
            .await?;
        if history.rows.len() > migrations.len() {
            return Err(Error("committed migrations were removed".into()));
        }
        for (row, migration) in history.rows.iter().zip(migrations) {
            if row.get::<String>(0)? != migration.name
                || row.get::<String>(1)? != migration.checksum
            {
                return Err(Error(format!(
                    "migration history/checksum mismatch at {}",
                    migration.name
                )));
            }
            if row.get::<bool>(2)? {
                return Err(Error(format!(
                    "{}: interrupted non-transactional migration requires operator repair",
                    migration.name
                )));
            }
        }
        let mut count = 0;
        for migration in &migrations[history.rows.len()..] {
            if d == Dialect::Sqlite && !migration.transactional {
                return Err(Error(
                    "non-transactional SQLite migration is forbidden".into(),
                ));
            }
            if migration.transactional && d == Dialect::Postgres {
                lease.batch("BEGIN").await?;
            }
            lease
                .query(
                    d,
                    "INSERT INTO _aor_migrations (name, checksum, dirty) VALUES ($1, $2, $3)",
                    &[
                        Value::Text(migration.name.clone()),
                        Value::Text(migration.checksum.clone()),
                        Value::Bool(!migration.transactional),
                    ],
                )
                .await?;
            lease.batch(&migration.sql).await?;
            if !migration.transactional {
                lease
                    .query(
                        d,
                        "UPDATE _aor_migrations SET dirty = $1 WHERE name = $2",
                        &[Value::Bool(false), Value::Text(migration.name.clone())],
                    )
                    .await?;
            }
            if migration.transactional && d == Dialect::Postgres {
                lease.batch("COMMIT").await?;
            }
            count += 1;
        }
        Ok(count)
    }
}
#[cfg(feature = "runtime")]
pub use runtime::apply;
