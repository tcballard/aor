#!/usr/bin/env python3
"""Real archive import/serve/restart with SQLite, and PostgreSQL when CI supplies it."""
import json, os, pathlib, socket, subprocess, tempfile, time, urllib.request, urllib.error, re, hashlib
ROOT=pathlib.Path(__file__).resolve().parents[1]
subprocess.run(['cargo','build','--locked','-p','aor-archive'],cwd=ROOT,check=True)
BIN=pathlib.Path(os.environ.get('CARGO_TARGET_DIR',str(ROOT/'target')))/'debug/aor-archive'
def run(args,ok=True):
    result=subprocess.run([str(BIN),*args],cwd=ROOT,capture_output=True,text=True)
    assert (result.returncode==0)==ok,(args,result.stderr)
    return result.stdout
with tempfile.TemporaryDirectory(prefix='aor-archive-db-') as tmp:
    tmp=pathlib.Path(tmp)
    modes=[['--sqlite',str(tmp/'archive.sqlite')]]
    if os.environ.get('AOR_TEST_DATABASE_URL'):
        os.environ['AOR_DATABASE_URL']=os.environ['AOR_TEST_DATABASE_URL']
        # CI shares a disposable service but needs a distinct archive database.
        subprocess.run(['psql',os.environ['AOR_DATABASE_URL'],'-v','ON_ERROR_STOP=1','-c','DROP TABLE IF EXISTS editions; DROP TABLE IF EXISTS _aor_migrations;'],check=True,capture_output=True)
        modes.append([])
    for mode in modes:
        fixture=tmp/'editions.json'
        edition={'slug':'checked-edition','title':'A <checked> edition','body':'<script>alert("unsafe")</script>\nSafe text.','published_at':'2026-09-30T09:00:00Z'}
        fixture.write_text(json.dumps([edition]))
        assert 'Applied 1' in run(['migrate',*mode])
        assert 'Applied 0' in run(['migrate',*mode])
        assert 'Imported 1' in run(['import',str(fixture),*mode])
        # Atomic validation failure leaves the existing edition unchanged.
        fixture.write_text(json.dumps([{**edition,'title':'Must not save'}, {**edition,'slug':'../bad'}]))
        run(['import',str(fixture),*mode],ok=False)
        for restart in range(2):
            with socket.socket() as sock:
                sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
            process=subprocess.Popen([str(BIN),'serve','--listen',f'127.0.0.1:{port}',*mode],cwd=ROOT,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
            try:
                url=f'http://127.0.0.1:{port}'
                for _ in range(100):
                    try:
                        index=urllib.request.urlopen(url,timeout=1).read().decode();break
                    except (OSError,urllib.error.URLError):
                        if process.poll() is not None:raise AssertionError(process.stderr.read().decode())
                        time.sleep(.05)
                else:raise AssertionError('archive never became ready')
                assert 'A &lt;checked&gt; edition' in index,index
                assert 'href="/editions/checked-edition"' in index,index
                css_path=re.search(r'/assets/archive\.([a-f0-9]{16})\.css',index)
                assert css_path,index
                css=urllib.request.urlopen(url+css_path.group())
                assert 'immutable' in css.headers['Cache-Control']
                assert hashlib.sha256(css.read()).hexdigest().startswith(css_path.group(1))
                body=urllib.request.urlopen(url+'/editions/checked-edition').read().decode()
                assert '&lt;script&gt;' in body and '<script>alert' not in body,body
                assert 'Must not save' not in body
                try:urllib.request.urlopen(url+'/editions/missing')
                except urllib.error.HTTPError as error:assert error.code==404
                else:raise AssertionError('missing edition did not return 404')
            finally:
                process.terminate();process.wait(timeout=5)
        edition['title']='Revised edition';fixture.write_text(json.dumps([edition]));run(['import',str(fixture),*mode])
        print('PASS: archive migrations, checked import, escaping, atomic validation, restart persistence, upsert:', 'SQLite' if mode else 'PostgreSQL')
