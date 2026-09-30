#!/usr/bin/env python3
"""Compile real downstream crates; malformed routes and missing Rust context fields fail."""
import pathlib,subprocess,tempfile
ROOT=pathlib.Path(__file__).resolve().parents[1]
cases={
 'duplicate-route-param':('use aor_router::*; fn main(){let _=route!(GET "/{id}/{id}" => handler);}', 'invalid or duplicate path parameter'),
 'unsafe-route-template':('use aor_router::*; fn main(){let _=route!(GET "/a?b" => handler);}', 'route must be a canonical absolute path'),
 'invalid-context-field':('#[derive(aor_tmpl::TemplateContext)] struct Context { value: std::fs::File } fn main(){}','TemplateContext'),
}
for name,(source,diagnostic) in cases.items():
 with tempfile.TemporaryDirectory(prefix='aor-compile-') as tmp:
  path=pathlib.Path(tmp);(path/'src').mkdir()
  (path/'Cargo.toml').write_text(f'[package]\nname="{name}"\nversion="0.0.0"\nedition="2024"\n[dependencies]\naor-router={{path="{ROOT}/crates/aor-router"}}\naor-tmpl={{path="{ROOT}/crates/aor-tmpl"}}\n')
  (path/'src/main.rs').write_text(source)
  result=subprocess.run(['cargo','check','--offline','--manifest-path',str(path/'Cargo.toml'),'--target-dir',str(ROOT/'target/compile-fail')],capture_output=True,text=True)
  assert result.returncode!=0 and diagnostic in result.stderr,(name,result.stderr)
  print(f'PASS: {name}')
