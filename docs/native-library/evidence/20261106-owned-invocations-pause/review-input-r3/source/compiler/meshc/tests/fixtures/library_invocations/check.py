#!/usr/bin/env python3
"""Actual generated C ABI; the old real HTTP response stays held until fresh result."""
import ctypes as C
import http.server
import json
import pathlib
import sqlite3
import sys
import threading
import time

P, U, I = C.c_void_p, C.c_uint64, C.c_int32
class Bytes(C.Structure):
    _fields_ = [('data', P), ('len', U)]
lib = C.CDLL(sys.argv[1])
legacy = '--legacy' in sys.argv
error = '--error' in sys.argv
outdir = pathlib.Path(sys.argv[2]); outdir.mkdir(parents=True, exist_ok=True)
def api(name, args, result=I):
    fn = getattr(lib, name); fn.argtypes = args; fn.restype = result
    return fn
create = api('mesh_authority_domain_create_v2', [C.POINTER(P)])
node = api('mesh_authority_generation_v2', [P,C.POINTER(P)])
bind = api('mesh_authority_bind_v2',[P,C.POINTER(P),U,P,C.POINTER(P)])
retire = api('mesh_authority_retire_v2',[P])
free = api('mesh_library_free_returned_bytes',[C.POINTER(Bytes)],None)
scoped = api('mesh_library_call_scoped_v2',[P,P,P,U,C.POINTER(Bytes)])
assert api('mesh_library_enable_protected_v2',[U])(1)==0
assert api('mesh_library_init',[])()==0
domain=P(); assert create(C.byref(domain))==0
owners=[]
for _ in range(2):
    generation,authority=P(),P()
    assert node(domain,C.byref(generation))==0
    assert bind(domain,(P*1)(generation),1,None,C.byref(authority))==0
    owners.append((generation,authority))
held,release=threading.Event(),threading.Event()
requests=[]
class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers.get('Content-Length','0')))
        requests.append(self.path)
        if self.path=='/hold':
            held.set(); assert release.wait(15)
        self.send_response(200); self.end_headers(); self.wfile.write(b'private-http')
    def log_message(self,*args): pass
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
threading.Thread(target=server.serve_forever,daemon=True).start()
url=f'http://127.0.0.1:{server.server_port}'
db=outdir/'effects.db'
with sqlite3.connect(db) as conn: conn.execute('CREATE TABLE effects (body TEXT)')
def value(status,output):
    data=C.string_at(output.data,output.len) if output.data else b''
    free(C.byref(output)); return status,data
if not legacy:
    take=api('mesh_library_invocation_take_v2',[P,C.POINTER(Bytes)])
    drop=api('mesh_library_invocation_release_v2',[P],None)
def invoke(which,symbol,payload):
    if legacy:
        output=Bytes(); return value(scoped(owners[which][1],getattr(lib,symbol),payload,len(payload),C.byref(output)),output)
    submit=api(symbol+'_submit_v2',[P,P,U,C.POINTER(P)])
    handle=P(); result=submit(owners[which][1],payload,len(payload),C.byref(handle))
    if result: return result,b''
    until=time.monotonic()+12
    while time.monotonic()<until:
        output=Bytes(); result=take(handle,C.byref(output))
        if result!=12:
            answer=value(result,output); drop(handle); return answer
        time.sleep(.001)
    raise AssertionError('scheduled invocation did not complete')
old=[]
worker=threading.Thread(target=lambda:old.append(invoke(0,'progress_error' if error else 'progress_old',f'{db}\n{url}/hold\n{url}/later'.encode())))
worker.start(); assert held.wait(10),'old export never reached actual HTTP'
fresh=invoke(1,'progress_load',str(db).encode())
progress=not release.is_set() and fresh==(0,b'loaded')
status=invoke(1,'progress_status',b'fresh-status')
retire(owners[0][0]); release.set(); worker.join(12)
assert not worker.is_alive()
with sqlite3.connect(db) as conn: rows=conn.execute('SELECT count(*) FROM effects').fetchone()[0]
facts={'legacy':legacy,'error':error,'fresh_status':fresh[0],'fresh_bytes':fresh[1].decode(),'status_status':status[0],'status_bytes':status[1].decode(),'fresh_completed_while_http_held':progress,'old_status':old[0][0],'old_bytes':old[0][1].decode(),'requests':requests,'rows':rows}
(outdir/'receipt.json').write_text(json.dumps(facts,indent=2)+'\n'); print(json.dumps(facts),flush=True)
server.shutdown()
for generation,authority in owners:
    api('mesh_authority_release_v2',[P],None)(authority)
    api('mesh_authority_generation_release_v2',[P],None)(generation)
api('mesh_authority_domain_release_v2',[P],None)(domain)
api('mesh_library_shutdown',[])()
assert progress and status==(0,b'fresh-status'),'fresh allowed load/status cannot finish while old HTTP held'
assert old==[(10,b'')] and rows==0 and requests==['/hold'],'old original authority escaped'
print('PASS')
