#!/usr/bin/env python3
"""Prove a failed Rust rebuild preserves the last-good process and then recovers."""
import http.client,pathlib,subprocess,time,json
ROOT=pathlib.Path(__file__).resolve().parents[1]
subprocess.run(['cargo','build','--locked','-p','aor-cli'],cwd=ROOT,check=True)
meta=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version=1'],cwd=ROOT,text=True))
binary=pathlib.Path(meta['target_directory'])/'debug/cargo-aor'
source=ROOT/'apps/archive/src/main.rs';original=source.read_bytes()
log=ROOT/'docs/evidence/dev-smoke.log'
def get(path):
 c=http.client.HTTPConnection('127.0.0.1',3000,timeout=2);c.request('GET',path);r=c.getresponse();result=(r.status,r.read());c.close();return result
def await_state(test,timeout=30):
 deadline=time.monotonic()+timeout
 while time.monotonic()<deadline:
  if process.poll() is not None:raise AssertionError(f'dev exited {process.returncode}; see {log}')
  try:
   if test():return
  except OSError:pass
  time.sleep(.1)
 raise AssertionError('timed out waiting for development state')
try:
 get('/healthz')
except OSError:pass
else:raise SystemExit('port 3000 is occupied; refusing to interfere')
with log.open('w') as output:
 process=subprocess.Popen([binary,'dev'],cwd=ROOT,stdout=output,stderr=subprocess.STDOUT)
 try:
  await_state(lambda:get('/healthz')[0]==200)
  # Initial watcher is installed just after the child starts.
  time.sleep(.3)
  source.write_bytes(original+b'\ncompile_error!("AOR_INTENTIONAL_DEV_TEST");\n')
  await_state(lambda:b'stale: build failed' in get('/_aor/dev-status')[1])
  assert get('/healthz')[0]==200
  source.write_bytes(original)
  await_state(lambda:get('/_aor/dev-status')[1]==b'current')
  assert get('/healthz')[0]==200
  print('PASS: failed rebuild retains last-good HTTP process, stale status is visible, corrected source restarts cleanly')
 finally:
  source.write_bytes(original);process.terminate()
  try:process.wait(timeout=5)
  except subprocess.TimeoutExpired:process.kill();process.wait()
