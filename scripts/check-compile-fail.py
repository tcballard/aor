#!/usr/bin/env python3
"""Compile real downstream crates; malformed routes and missing Rust context fields fail."""
import pathlib,subprocess,tempfile,os
ROOT=pathlib.Path(__file__).resolve().parents[1]
cases={
 'generated-job-incomplete':('#[path="broken_job.rs"] mod generated; fn main(){}','AOR_JOB_INCOMPLETE'),

 'detached-mail-enqueue':('async fn bad(lease:&mut aor_db::Lease,outbox:&aor_mail::Outbox){let _=outbox.enqueue_in(lease,"to@example.test","verify",2000,1000).await;} fn main(){}','mismatched types'),
 'mail-token-wire':('fn main(){fn wire<T:serde::Serialize>(){} wire::<aor_mail::IssuedToken>();}', 'Serialize'),

 'detached-enqueue':('async fn bad(lease:&mut aor_db::Lease, job:&aor_registry::manifests::ValidateManifestV1){let _=aor_jobs::enqueue_in(lease,job,"registry",100,aor_jobs::Options::default()).await;} fn main(){}','mismatched types'),
 'forged-job-claim':('fn main(){let _=aor_jobs::Claim{};}', 'private fields'),
 'missing-job-idempotency':('#[derive(serde::Serialize,serde::Deserialize)] struct Missing; impl aor_jobs::Job for Missing {const NAME:&\'static str="missing";const VERSION:i64=1;fn key(&self)->String{String::new()}} fn main(){}', 'IDEMPOTENCY'),

 'forged-principal':('fn main(){let _=aor_session::Principal{user_id:aor_db::Uuid::new_v4(),epoch:1,credential_hash:String::new(),scopes:None};}', 'private'),
 'forged-scope':('use aor_policy::*; enum Plugin{} impl Resource for Plugin{const NAME:&\'static str="plugins";} fn main(){let _=Authorized::<Plugin,Read>{};}', 'private fields'),
 'owner-input':('fn main(){let _=aor_registry::plugins::Input{name:"x".into(),owner_id:aor_db::Uuid::new_v4()};}', 'has no field named'),

 'two-body-extractors':('use aor_router::*; #[handler] async fn bad(_: Json<String>, _: Body)->Result<aor_http::Response,AppError>{unimplemented!()} fn main(){}','consume the request body only once'),
 'unknown-extractor':('use aor_router::*; #[handler] async fn bad(_: String)->Result<aor_http::Response,AppError>{unimplemented!()} fn main(){}','unsupported extractor'),
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
  (path/'Cargo.toml').write_text(f'[package]\nname="{name}"\nversion="0.0.0"\nedition="2024"\n[dependencies]\nserde={{version="1",features=["derive"]}}\naor-router={{path="{ROOT}/crates/aor-router"}}\naor-http={{path="{ROOT}/crates/aor-http"}}\naor-tmpl={{path="{ROOT}/crates/aor-tmpl"}}\naor-db={{path="{ROOT}/crates/aor-db"}}\naor-session={{path="{ROOT}/crates/aor-session"}}\naor-policy={{path="{ROOT}/crates/aor-policy"}}\naor-registry={{path="{ROOT}/apps/registry"}}\naor-jobs={{path="{ROOT}/crates/aor-jobs"}}\naor-mail={{path="{ROOT}/crates/aor-mail"}}\n')
  (path/'src/main.rs').write_text(source)
  for directory,ddl in [('migrations','CREATE TABLE editions (id UUID PRIMARY KEY, title TEXT NOT NULL);'),('local_migrations','CREATE TABLE editions (id TEXT PRIMARY KEY);'),('bad_migrations','-- invalid DDL\nCREATE VIEW nope AS SELECT 1;')]:
   (path/directory).mkdir();(path/directory/'001_bad.sql').write_text(ddl)
  if name=='generated-job-incomplete':
   scaffold=['cargo','run','--quiet','--locked','-p','aor-cli','--','scaffold','job','Broken','--root',str(path)]
   preview=subprocess.run([*scaffold,'--dry-run'],cwd=ROOT,capture_output=True,text=True,check=True)
   assert not (path/'src/broken_job.rs').exists()
   applied=subprocess.run(scaffold,cwd=ROOT,capture_output=True,text=True,check=True)
   import json
   expected=json.loads(preview.stdout)['files']; actual=json.loads(applied.stdout)['files']
   assert expected==actual and all((path/p).read_text()==text for p,text in expected.items())
  result=subprocess.run(['cargo','check','--offline','--manifest-path',str(path/'Cargo.toml'),'--target-dir',str(pathlib.Path(os.environ.get('CARGO_TARGET_DIR',str(ROOT/'target')))/'compile-fail')],capture_output=True,text=True)
  assert result.returncode!=0 and diagnostic in result.stderr,(name,result.stderr)
  print(f'PASS: {name}')
