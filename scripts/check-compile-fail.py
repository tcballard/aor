#!/usr/bin/env python3
"""Compile real downstream crates; malformed routes and missing Rust context fields fail."""
import pathlib,subprocess,tempfile
ROOT=pathlib.Path(__file__).resolve().parents[1]
cases={
 'unknown-sql-column':('aor_db::sql!(Broken, postgres, "migrations", "SELECT never_created FROM editions"); fn main(){}','unknown column never_created'),
 'sql-star':('aor_db::sql!(Broken, postgres, "migrations", "SELECT * FROM editions"); fn main(){}','SELECT * is forbidden'),
 'sql-parameter-type':('aor_db::sql!(Q, postgres, "migrations", "SELECT id FROM editions WHERE id = $1"); async fn bad(c: &mut impl aor_db::Executor){let _=Q::query(c, "wrong".to_string()).await;} fn main(){}','mismatched types'),
 'unsupported-ddl':('aor_db::sql!(Broken, postgres, "bad_migrations", "SELECT id FROM editions"); fn main(){}','001_bad.sql:line 2'),
 'sqlite-row-lock':('aor_db::sql!(Broken, sqlite, "local_migrations", "SELECT id FROM editions FOR UPDATE"); fn main(){}','row locks require PostgreSQL'),
 'duplicate-route-param':('use aor_router::*; fn main(){let _=route!(GET "/{id}/{id}" => handler);}', 'invalid or duplicate path parameter'),
 'unsafe-route-template':('use aor_router::*; fn main(){let _=route!(GET "/a?b" => handler);}', 'route must be a canonical absolute path'),
 'invalid-context-field':('#[derive(aor_tmpl::TemplateContext)] struct Context { value: std::fs::File } fn main(){}','TemplateContext'),
}
for name,(source,diagnostic) in cases.items():
 with tempfile.TemporaryDirectory(prefix='aor-compile-') as tmp:
  path=pathlib.Path(tmp);(path/'src').mkdir()
  (path/'Cargo.toml').write_text(f'[package]\nname="{name}"\nversion="0.0.0"\nedition="2024"\n[dependencies]\naor-router={{path="{ROOT}/crates/aor-router"}}\naor-tmpl={{path="{ROOT}/crates/aor-tmpl"}}\naor-db={{path="{ROOT}/crates/aor-db"}}\n')
  (path/'src/main.rs').write_text(source)
  for directory,ddl in [('migrations','CREATE TABLE editions (id UUID PRIMARY KEY, title TEXT NOT NULL);'),('local_migrations','CREATE TABLE editions (id TEXT PRIMARY KEY);'),('bad_migrations','-- invalid DDL\nCREATE VIEW nope AS SELECT 1;')]:
   (path/directory).mkdir();(path/directory/'001_bad.sql').write_text(ddl)
  result=subprocess.run(['cargo','check','--offline','--manifest-path',str(path/'Cargo.toml'),'--target-dir',str(ROOT/'target/compile-fail')],capture_output=True,text=True)
  assert result.returncode!=0 and diagnostic in result.stderr,(name,result.stderr)
  print(f'PASS: {name}')
