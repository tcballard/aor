#!/usr/bin/env python3
"""Launch the real archive binary and verify HTTP, templates, theme and dev reload."""
import http.client,json,os,pathlib,socket,subprocess,tempfile,time
ROOT=pathlib.Path(__file__).resolve().parents[1]
def main():
    subprocess.run(['cargo','build','--locked','-p','aor-archive'],cwd=ROOT,check=True)
    metadata=json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version=1'],cwd=ROOT,text=True))
    binary=pathlib.Path(metadata['target_directory'])/'debug/aor-archive'
    with socket.socket() as s:s.bind(('127.0.0.1',0));port=s.getsockname()[1]
    process=subprocess.Popen([binary,'serve','--listen',f'127.0.0.1:{port}','--dev'],cwd=ROOT,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
    def get(path,headers=None):
        c=http.client.HTTPConnection('127.0.0.1',port,timeout=3);c.request('GET',path,headers=headers or {});r=c.getresponse();result=(r.status,r.read(),dict(r.getheaders()));c.close();return result
    template=ROOT/'apps/archive/templates/index.html';original=template.read_bytes()
    try:
        for _ in range(100):
            if process.poll() is not None:raise AssertionError(process.stderr.read().decode())
            try:
                if get('/healthz')[0]==200:break
            except OSError:pass
            time.sleep(.05)
        else:raise AssertionError('server did not become ready')
        status,body,headers=get('/');assert status==200 and b'Fixing Everything' in body
        assert headers['X-Content-Type-Options']=='nosniff'
        assert get('/healthz')[1]==b'{"status":"ok","public_ready":false}'
        assert get('/_aor/theme.css')[0]==200
        assert get('/../../etc/passwd')[0]==400
        assert get('/%2e%2e/etc/passwd')[0]==400
        assert get('/',{'Cookie':'session=attacker'})[0]==503
        assert get('/_aor/reload.js')[0]==200
        c=http.client.HTTPConnection('127.0.0.1',port,timeout=3);c.request('GET','/_aor/reload');r=c.getresponse();assert r.status==200
        first=r.readline();assert first.startswith(b'data: ');r.readline()
        template.write_bytes(original.replace(b'BUILD NOTE 001',b'LIVE RELOAD VERIFIED'))
        deadline=time.monotonic()+3;changed=False
        while time.monotonic()<deadline:
            line=r.readline()
            if line.startswith(b'data: ') and line!=first:changed=True;break
        assert changed,'inotify did not trigger an SSE update';c.close()
        assert b'LIVE RELOAD VERIFIED' in get('/')[1]
        template.write_text('<h1>{{ missing_field }}</h1>')
        status,body,_=get('/');assert status==500 and b'unknown field' in body
        print('PASS: HTTP, embedded assets, typed template reload, theme CSS, request boundaries, SSE reload, development errors')
    finally:
        template.write_bytes(original);process.terminate()
        try:process.wait(timeout=4)
        except subprocess.TimeoutExpired:process.kill();process.wait()
if __name__=='__main__':main()
