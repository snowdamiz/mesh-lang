#!/usr/bin/env python3
"""Generated C ABI + actual loopback HTTP/SQLite. One runtime per subprocess.

--baseline exercises the same sinks through V1 with only an outer synthetic
retirement flag. It intentionally fails the retirement assertions on 8256d5d.
This embedding fixture is not an iOS/Android/macOS lifecycle qualification.
"""
import ctypes as C
import http.server
import hashlib
import subprocess
import json
import pathlib
import sqlite3
import sys
import tempfile
import threading

lib = C.CDLL(sys.argv[1])
case = sys.argv[2]
baseline = '--baseline' in sys.argv or case.startswith('v1-')
stack_probe = next((C.CDLL(arg.split('=',1)[1]).native_stack_probe for arg in sys.argv if arg.startswith('--stack-probe=')), None)
stack_results = []
P = C.c_void_p
U = C.c_uint64
I = C.c_int32

class Bytes(C.Structure):
    _fields_ = [('data', P), ('len', U)]

Callback = C.CFUNCTYPE(I, P, P, U, P, U, C.POINTER(U))
Lifetime = C.CFUNCTYPE(None, P)
class V1(C.Structure):
    _fields_ = [('abi', C.c_uint32), ('size', C.c_uint32), ('context', P)] + [
        (name, Callback) for name in ['put', 'get', 'delete', 'push', 'background',
                                    'network', 'monotonic', 'wall', 'log']]
class V2(C.Structure):
    _fields_ = [('abi', C.c_uint32), ('size', C.c_uint32), ('callbacks', V1),
                ('retain', Lifetime), ('release', Lifetime)]

def api(name, args, result=I):
    fn = getattr(lib, name)
    fn.argtypes = args
    fn.restype = result
    return fn

init = api('mesh_library_init', [])
shutdown = api('mesh_library_shutdown', [])
free = api('mesh_library_free_returned_bytes', [C.POINTER(Bytes)], None)
register = api('mesh_library_register_host_callbacks', [C.POINTER(V1)])
ready = threading.Event()
resume = threading.Event()
released = threading.Event()
requests = []
commit_hook_calls = []
mutations = []
records = {}
lifetime = [0, 0]
observed_ids = []
network_markers = []
refusal_handles = []
retired = False
retire_during_put = case == 'accepted-store'

def wait(event):
    assert event.wait(10), 'barrier timed out'

def host(kind):
    @Callback
    def callback(context, inp, size, out, cap, written):
        if not baseline:
            observed_ids.append(current_id())
            if stack_probe: stack_results.append(stack_probe())
        data = C.string_at(inp, size)
        answer = b'ok'
        if kind == 'network':
            network_markers.append(data.decode())
            if case in ['request-refusal', 'stream-refusal']:
                # Run on the generated invocation's real callback stack while
                # its SQLite transaction is open. Never dispatch these requests.
                string = api('mesh_string_new', [P, U], P)
                method = string(b'post', 4)
                target = (url+'/must-not-arrive').encode()
                target = string(target, len(target))
                build = api('mesh_http_build', [P, P], U)
                send = api('mesh_http_send', [U], P)
                stream = api('mesh_http_stream', [U, P, P], C.c_int64)
                for _ in range(4100):
                    handle = build(method, target)
                    refusal_handles.append(handle)
                    if not handle: break
                    if case == 'request-refusal': send(handle)
                    else: stream(handle, None, None)
                written[0] = 0
                return 0
            if case == 'terminate': answer = str(path).encode()
            ready.set()
            if not resume.wait(10):
                return 90
        if kind == 'put':
            if case in ['v1-6','v1-8']: return int(case[3:])
            if case == 'v1-output':
                written[0] = cap + 1
                return 0
            mutations.append('put')
            if len(data) >= 4:
                keylen = int.from_bytes(data[:4], 'big')
                if 0 < keylen <= len(data)-4:
                    records[data[4:4+keylen]] = data[4+keylen:]
            if retire_during_put or (case == 'accepted-nonce' and len(mutations) == 2):
                retire()
                if not baseline: assert local_inflight(generation) == 1
        if kind == 'get':
            if data not in records:
                return 2
            answer = records[data]
        if kind == 'delete':
            mutations.append('delete')
            records.pop(data, None)
        if len(answer) > cap:
            answer = b''
        if answer:
            C.memmove(out, answer, len(answer))
        written[0] = len(answer)
        return 0
    return callback

callbacks = [host(k) for k in ['put','get','delete','push','background','network','monotonic','wall','log']]
if case == 'v1-missing': callbacks[0] = Callback()
@Lifetime
def retain(_):
    lifetime[0] += 1
@Lifetime
def release(_):
    if not baseline: current_id()  # Finalizers may inspect TLS without borrow/lock reentry.
    lifetime[1] += 1
    released.set()
v1 = V1(1, C.sizeof(V1), None, *callbacks)
v2 = V2(2, C.sizeof(V2), v1, retain, release)
domain, generation, authority = P(), P(), P()
extra_nodes = []

if not baseline:
    current_id = api('mesh_library_current_invocation_v2', [], U)
    local_inflight = api('mesh_authority_local_inflight_v2', [P], U)
    capabilities = api('mesh_library_capabilities_v2', [], U)
    enable = api('mesh_library_enable_protected_v2', [U])
    for unsupported in [2, 4, 8, 16, 32, 1 << 63]:
        assert enable(1 | unsupported) != 0, 'unsupported activation accepted'
    create = api('mesh_authority_domain_create_v2', [C.POINTER(P)])
    new_node = api('mesh_authority_generation_v2', [P, C.POINTER(P)])
    bind = api('mesh_authority_bind_v2', [P, C.POINTER(P), U, C.POINTER(V2), C.POINTER(P)])
    node_retire = api('mesh_authority_retire_v2', [P])
    domain_release = api('mesh_authority_domain_release_v2', [P], None)
    node_release = api('mesh_authority_generation_release_v2', [P], None)
    authority_release = api('mesh_authority_release_v2', [P], None)
    scoped = api('mesh_library_call_scoped_v2', [P, P, P, U, C.POINTER(Bytes)])
    assert capabilities() & 1
    if case == 'unsupported':
        assert init() == 0
        out = Bytes()
        fn = api('authority_error', [P, U, C.POINTER(Bytes)])
        assert fn(None, 0, C.byref(out)) == 9 and out.len > 0, 'failed activation changed V1'
        free(C.byref(out))
        assert shutdown() == 0
        print('PASS unsupported')
        sys.exit(0)
    assert enable(1) == 0
    assert create(C.byref(domain)) == 0
    assert new_node(domain, C.byref(generation)) == 0
    if case == 'ancestry':
        for _ in range(31):
            extra = P()
            assert new_node(domain, C.byref(extra)) == 0
            extra_nodes.append(extra)
    node_list = [generation, *extra_nodes]
    nodes = (P * len(node_list))(*node_list)
    assert bind(domain, nodes, len(node_list), C.byref(v2), C.byref(authority)) == 0
    if case == 'ancestry':
        rejected = P(123)
        overflow = (P * 33)(*[generation] * 33)
        assert bind(domain, overflow, 33, C.byref(v2), C.byref(rejected)) != 0 and not rejected
        generation, extra_nodes[15] = extra_nodes[15], generation
    if case == 'generations':
        for _ in range(1024):
            transient = P()
            assert new_node(domain, C.byref(transient)) == 0
            assert node_retire(transient) == 0
            node_release(transient)
assert init() == 0
assert register(C.byref(v1)) == 0

def retire():
    global retired
    retired = True
    if not baseline:
        assert node_retire(generation) == 0

def invoke(symbol, payload=b'', scoped_call=True):
    fn = api(symbol, [P, U, C.POINTER(Bytes)])
    out = Bytes()
    if not baseline and scoped_call:
        status = scoped(authority, C.cast(fn, P), payload, len(payload), C.byref(out))
    else:
        status = fn(payload, len(payload), C.byref(out))
    data = C.string_at(out.data, out.len) if out.len else b''
    free(C.byref(out))
    return status, data

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def do_POST(self):
        body = self.rfile.read(int(self.headers['Content-Length']))
        requests.append((self.path, body.decode()))
        if self.path == '/hold':
            ready.set()
            wait(resume)
        self.send_response(200)
        self.send_header('Content-Length', '2')
        self.end_headers()
        self.wfile.write(b'ok')

# A real SQLite commit hook parks INSIDE sqlite3_step, after the runtime's
# irrevocable acceptance. It is test embedding instrumentation, not a Mesh UDF.
CommitHook = C.CFUNCTYPE(I, P)
AutoExtension = C.CFUNCTYPE(I, P, P, P)
@CommitHook
def commit_hook(_):
    commit_hook_calls.append(1)
    ready.set()
    if not resume.wait(10): return 1
    return 0
@AutoExtension
def sqlite_extension(db, error, routines):
    api('sqlite3_commit_hook', [P, CommitHook, P], P)(db, commit_hook, None)
    return 0
if case == 'accepted-commit':
    assert api('sqlite3_auto_extension', [AutoExtension])(sqlite_extension) == 0

server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
server_thread = threading.Thread(target=server.serve_forever, daemon=True)
server_thread.start()
url = f'http://127.0.0.1:{server.server_port}'
results = []
with tempfile.TemporaryDirectory() as directory:
    path = pathlib.Path(directory)/'effects.db'
    with sqlite3.connect(path) as db:
        db.execute('CREATE TABLE effects (body TEXT)')
    if case in ['write','query-write','schema','commit','raw-commit','release','control','accepted-commit','generations','hot-journal','ancestry']:
        mode = 'write' if case in ['control','generations','hot-journal','ancestry'] else case
        symbol = 'authority_sql'
        payload = f'{path}\n{mode}\n{url}/hold'.encode()
    elif case == 'http':
        symbol, payload = 'authority_http', f'{url}/hold\n{url}/later'.encode()
    elif case in ['request-refusal','stream-refusal']:
        status, data = invoke('authority_sql', f'{path}\ncommit\n{url}/unused'.encode())
        assert status == 10 and not data
        assert len(refusal_handles) == 4100 and all(refusal_handles), \
            f'request registry exhausted after {sum(bool(h) for h in refusal_handles)} refusals'
        assert requests == [], 'refused request dispatched'
        # A new original invocation must still be able to construct and send.
        assert invoke('authority_http', f'{url}/fresh\n{url}/later'.encode()) == (0,b'done')
        assert requests == [('/fresh','exact-attempt'),('/later','exact-attempt')]
        with sqlite3.connect(path) as db:
            assert db.execute('SELECT count(*) FROM effects').fetchone()[0] == 0
        symbol = None
    elif case in ['store','accepted-store','registration','v1-6','v1-8','v1-output','v1-missing']:
        symbol, payload = 'authority_store', b'record'
    elif case in ['nonce','accepted-nonce']:
        symbol, payload = 'authority_nonce', b'plain'
    elif case in ['actor','timer']:
        symbol, payload = ('authority_actor' if case == 'actor' else 'authority_timer'), b'actor-record'
    elif case == 'terminate':
        symbol, payload = 'authority_terminate', b''
    elif case == 'missing':
        status, data = invoke('authority_store', b'x', scoped_call=False)
        assert status != 0 and not data and not mutations
        symbol = None
    elif case == 'gc':
        status, data = invoke('authority_gc', b'owned-input')
        assert (status, data) == (0,b'owned-input')
        symbol = None
    elif case == 'panic':
        status, data = invoke('authority_panic')
        assert status == 4 and not data
        symbol = None
    elif case == 'error':
        status, data = invoke('authority_error')
        assert status != 0 and not data, 'private application error crossed protected ABI'
        symbol = None
    else:
        raise AssertionError(case)
    if symbol:
        worker = threading.Thread(target=lambda: results.append(invoke(symbol, payload)))
        worker.start()
        wait(ready)
        before = list(mutations)
        if case == 'hot-journal':
            # Spill dirty pages and exit without rollback, leaving a genuine hot
            # rollback journal. Do not open SQLite to observe it: that recovers it.
            crash = """import sqlite3,sys,os
c=sqlite3.connect(sys.argv[1]);c.execute('PRAGMA cache_size=5');c.execute('PRAGMA synchronous=FULL');c.execute('BEGIN IMMEDIATE')
for i in range(128): c.execute('INSERT INTO effects VALUES (?)',('x'*8192,))
os._exit(0)
"""
            subprocess.run([sys.executable, '-c', crash, str(path)], check=True)
            journal = pathlib.Path(str(path)+'-journal')
            assert journal.exists() and journal.read_bytes()[:8] != bytes(8), 'not a hot journal'
            before_recovery = (hashlib.sha256(path.read_bytes()).hexdigest(), hashlib.sha256(journal.read_bytes()).hexdigest())
        if case == 'registration':
            replacement_callbacks = [host('successor-'+str(i)) for i in range(9)]
            replacement = V1(1, C.sizeof(V1), None, *replacement_callbacks)
            assert register(C.byref(replacement)) == 0
        if case not in ['control', 'accepted-store', 'accepted-nonce', 'generations', 'registration','terminate','v1-6','v1-8','v1-output','v1-missing']:
            retire()
        if not baseline:
            assert local_inflight(generation) == (1 if case == 'accepted-commit' else 0)
        if case in ['actor','timer','terminate'] and not baseline:
            worker.join(10)
            assert not worker.is_alive()
            authority_release(authority)
            assert not released.is_set(), 'captured callback freed while actor blocked'
            authority = P()
        resume.set()
        worker.join(10)
        assert not worker.is_alive(), 'invocation did not finish'
        if case in ['actor','timer','terminate'] and not baseline:
            wait(released)
        if case == 'hot-journal':
            assert journal.exists(), 'unadmitted preparation deleted hot journal'
            after_recovery = (hashlib.sha256(path.read_bytes()).hexdigest(), hashlib.sha256(journal.read_bytes()).hexdigest())
            assert after_recovery == before_recovery, 'unadmitted preparation rewrote database/journal'
        with sqlite3.connect(path) as db:
            count = db.execute('SELECT count(*) FROM effects').fetchone()[0]
            later = db.execute("SELECT count(*) FROM sqlite_master WHERE name='later'").fetchone()[0]
        facts = {'case':case, 'baseline':baseline,'status':results[0][0], 'rows':count,
                 'later_schema':later,'requests':requests,'mutations':mutations,
                 'mutations_before_retire':before,'retired':retired}
        print(json.dumps(facts), flush=True)
        if case in ['v1-6','v1-8']:
            assert results[0] == (9, f'host_callback_failed:1:{case[3:]}'.encode()), results
        elif case == 'v1-missing':
            assert results[0] == (9,b'host_callback_missing'), results
        elif case == 'v1-output':
            assert results[0] == (9,b'host_callback_output_too_large'), results
        elif case == 'registration':
            assert mutations == ['put','delete'] and results[0][0] == 0
        elif case == 'accepted-commit':
            assert count == 1 and commit_hook_calls == [1], 'accepted commit did not finish'
            assert results[0][0] != 0 and not results[0][1]
        elif case in ['control','generations']:
            assert count == 1 and results[0][0] == 0
        else:
            assert count == 0 and later == 0, 'unadmitted SQL effect reached SQLite'
            if case == 'http':
                assert requests == [('/hold','exact-attempt')], 'unadmitted HTTP attempt'
            if case in ['store','nonce','actor','timer']:
                assert mutations == before, 'unadmitted secure-store/nonce effect'
            if case == 'accepted-nonce':
                assert mutations == ['put','put']
                record = records[b'mesh/storage-key/v2']
                assert int.from_bytes(record[-8:], 'big') == 1, 'admitted nonce was not burned'
            if case == 'accepted-store':
                assert mutations == ['put'], 'accepted write must finish; next write must refuse'
            if case not in ['actor','timer','terminate']:
                assert results[0][0] != 0 and not results[0][1], 'retired/private error bytes released'
if case in ['actor','timer','terminate']:
    assert network_markers == [case+'-ready'], network_markers
if not baseline:
    assert all(observed_ids), observed_ids
    assert all(stack_results), 'Host callback did not run on native thread stack'
    assert len(set(observed_ids)) <= 1, observed_ids
    assert local_inflight(generation) == 0
    if authority:
        authority_release(authority)
    node_release(generation)
    for node in extra_nodes: node_release(node)
    domain_release(domain)
    wait(released)
    assert lifetime == [1,1], lifetime
assert shutdown() == 0
server.shutdown()
print('PASS',case,flush=True)
