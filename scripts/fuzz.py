#!/usr/bin/env python3
"""Run instrumented fuzz targets and record actual child CPU time on a source hash.
Build CPU time is excluded. This is evidence capture, not a public-readiness claim.
"""
import argparse, datetime, hashlib, json, os, pathlib, resource, subprocess, sys
ROOT=pathlib.Path(__file__).resolve().parents[1]
def source_hash(relative):
    h=hashlib.sha256()
    for p in sorted((ROOT/relative).rglob('*.rs')):
        h.update(p.relative_to(ROOT).as_posix().encode()+b'\0'+p.read_bytes()+b'\0')
    return h.hexdigest()
def main():
    p=argparse.ArgumentParser();p.add_argument('--seconds',type=int,default=60);p.add_argument('--no-leak-check',action='store_true',help='local sandbox smoke only; ineligible for release hours');p.add_argument('--target',choices=['request','chunked','tokens','template'],action='append');args=p.parse_args()
    if not 1<=args.seconds<=18000:p.error('seconds must be between 1 and 18000')
    snapshots={source:source_hash(source) for source in ['crates/aor-http/src','crates/aor-tmpl/src']}
    subprocess.run(['cargo','+nightly','fuzz','build'],cwd=ROOT,check=True)
    assert all(source_hash(source)==h for source,h in snapshots.items()), 'source changed during fuzz build'
    host=subprocess.check_output(['rustc','+nightly','-vV'],text=True).split('host: ')[1].splitlines()[0]
    evidence=ROOT/'docs/evidence/fuzz.json';report=json.loads(evidence.read_text()) if evidence.exists() else {'schema_version':1,'runs':[]}
    for target in args.target or ['request','chunked','tokens','template']:
        binary=ROOT/'fuzz/target'/host/'release'/target
        source='crates/aor-tmpl/src' if target=='template' else 'crates/aor-http/src'
        revision=snapshots[source];log=ROOT/'docs/evidence'/f'fuzz-{target}-{len(report["runs"]):04d}.log'
        before=resource.getrusage(resource.RUSAGE_CHILDREN)
        with log.open('w') as output:
            proc=subprocess.run([str(binary),str(ROOT/'fuzz/corpus'/target),f'-max_total_time={args.seconds}','-max_len=8192','-timeout=5','-rss_limit_mb=256','-print_final_stats=1','-verbosity=0'],cwd=ROOT,stdout=output,stderr=subprocess.STDOUT,env={**os.environ,'ASAN_OPTIONS':'quarantine_size_mb=16:malloc_context_size=15'+(':detect_leaks=0' if args.no_leak_check else '')})
        after=resource.getrusage(resource.RUSAGE_CHILDREN)
        cpu=(after.ru_utime+after.ru_stime)-(before.ru_utime+before.ru_stime)
        run={'target':target,'source_sha256':revision,'cpu_seconds':cpu,'crash_free':proc.returncode==0 and source_hash(source)==revision,'exit_code':proc.returncode,'release_eligible':not args.no_leak_check,'completed_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'log':log.relative_to(ROOT).as_posix(),'toolchain':subprocess.check_output(['rustc','+nightly','--version'],text=True).strip()}
        report['runs'].append(run);evidence.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(run),flush=True)
        if proc.returncode:sys.exit(proc.returncode)
if __name__=='__main__':main()
