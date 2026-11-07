"""Read-only artifact probe: native scope -> existing Repo.transaction boundary.

No compiler, app or source-tree changes. The peer binds only owned loopback.
This uses a native test export, not a generated scheduler entry.
"""
import ctypes as C
import hashlib
import json
import os
from pathlib import Path
import socket
import struct
import threading

os.environ['MESH_SCHEDULER_MIN_WORKERS'] = '1'
os.environ['MESH_SCHEDULER_MAX_WORKERS'] = '1'
ROOT = Path(__file__).resolve().parent
LIB = ROOT.parent / 'generated-abi-1794016967512686000/libprogress.dylib'
before = hashlib.sha256(LIB.read_bytes()).hexdigest()
lib = C.CDLL(str(LIB))
P, U, I = C.c_void_p, C.c_uint64, C.c_int32
class Result(C.Structure):
    _fields_ = [('tag', C.c_ubyte), ('value', P)]
class Bytes(C.Structure):
    _fields_ = [('data', P), ('len', U)]
def api(name, args, result=I):
    fn = getattr(lib, name)
    fn.argtypes, fn.restype = args, result
    return fn
def error(ptr):
    result = Result.from_address(ptr)
    assert result.tag == 1
    length = U.from_address(result.value).value
    return C.string_at(result.value + 8, length).decode()
string = api('mesh_string_new', [P, U], P)
def text(value):
    value = value.encode()
    return string(value, len(value))
assert api('mesh_library_init', [])() == 0
listener = socket.socket()
listener.bind(('127.0.0.1', 0))
listener.listen()
listener.settimeout(3)
url = text(f'postgres://u@127.0.0.1:{listener.getsockname()[1]}/d?sslmode=disable')
opened = api('mesh_pool_open', [P, C.c_int64, C.c_int64, C.c_int64], P)(url, 0, 1, 50)
result = Result.from_address(opened)
assert result.tag == 0
pool = U.from_address(result.value).value
domain, generation, authority = P(), P(), P()
assert api('mesh_authority_domain_create_v2', [C.POINTER(P)])(C.byref(domain)) == 0
assert api('mesh_authority_generation_v2', [P, C.POINTER(P)])(domain, C.byref(generation)) == 0
assert api('mesh_authority_bind_v2', [P, C.POINTER(P), U, P, C.POINTER(P)])(domain, (P*1)(generation), 1, None, C.byref(authority)) == 0
scoped = api('mesh_library_call_scoped_v2', [P, P, P, U, C.POINTER(Bytes)])
pg = api('mesh_pg_connect', [P], P)
repo = api('mesh_repo_transaction', [U, P, P], P)
facts = {'artifact': str(LIB), 'sha256_before': before, 'peer_frames': [], 'probe_kind': 'native scoped export; not generated scheduled entry'}
NativeExport = C.CFUNCTYPE(I, P, U, C.POINTER(Bytes))
@NativeExport
def direct(_input, _size, _output):
    facts['direct_pg_error'] = error(pg(url))
    return 0
assert scoped(authority, direct, None, 0, C.byref(Bytes())) == 0
listener.settimeout(.05)
try:
    unexpected, _ = listener.accept()
    unexpected.close()
    raise AssertionError('direct Pg.connect reached the peer')
except socket.timeout:
    facts['direct_pg_no_connection'] = True
listener.settimeout(3)
done = threading.Event()
def exact(sock, count):
    value = b''
    while len(value) < count:
        more = sock.recv(count - len(value))
        if not more:
            raise EOFError()
        value += more
    return value
def peer():
    try:
        sock, _ = listener.accept()
        with sock:
            sock.settimeout(3)
            size = struct.unpack('!I', exact(sock, 4))[0]
            startup = exact(sock, size - 4)
            facts['peer_frames'].append({'kind': 'startup', 'bytes': len(startup)})
            sock.sendall(b'R\0\0\0\x08\0\0\0\0Z\0\0\0\x05I')
            done.wait(3)
    except Exception as failure:
        facts['peer_error'] = repr(failure)
thread = threading.Thread(target=peer)
thread.start()
@NativeExport
def transaction(_input, _size, _output):
    # Pg.transaction must refuse before invoking this null test callback.
    facts['repo_error'] = error(repo(pool, None, None))
    return 0
assert scoped(authority, transaction, None, 0, C.byref(Bytes())) == 0
query = api('mesh_pool_query', [U, P, P], P)
params = api('mesh_list_new', [], P)()
facts['subsequent_unscoped_query_error'] = error(query(pool, text('SELECT 1'), params))
done.set()
thread.join(4)
listener.close()
api('mesh_pool_close', [U], None)(pool)
for name, arg in [('mesh_authority_release_v2', authority), ('mesh_authority_generation_release_v2', generation), ('mesh_authority_domain_release_v2', domain)]:
    api(name, [P], None)(arg)
assert api('mesh_library_shutdown', [])() == 0
facts['sha256_after'] = hashlib.sha256(LIB.read_bytes()).hexdigest()
assert facts['sha256_after'] == before
assert facts['direct_pg_error'] == 'native_effect_refused'
assert facts['repo_error'] == 'native_effect_refused'
assert len(facts['peer_frames']) == 1
assert facts['subsequent_unscoped_query_error'] == 'pool checkout timeout'
(ROOT / 'repo-refusal-probe.json').write_text(json.dumps(facts, indent=2) + '\n')
print(json.dumps(facts, indent=2))
