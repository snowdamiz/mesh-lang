"""Native companion bounds/state contract against actual generated thunks."""
import ctypes as C
import json
from pathlib import Path
import sys
import time

P, U, I = C.c_void_p, C.c_uint64, C.c_int32
class Bytes(C.Structure):
    _fields_ = [('data', P), ('len', U)]
lib = C.CDLL(sys.argv[1])
def api(name, args, result=I):
    function = getattr(lib, name)
    function.argtypes, function.restype = args, result
    return function
small = api('mesh_fixture_small_output_submit_v2', [P,P,U,C.POINTER(P)])
large = api('mesh_fixture_large_submit_v2', [P,P,U,C.POINTER(P)])
take = api('mesh_library_invocation_take_v2', [P,C.POINTER(Bytes)])
cancel = api('mesh_library_invocation_cancel_v2', [P])
release = api('mesh_library_invocation_release_v2', [P], None)
free = api('mesh_library_free_returned_bytes', [C.POINTER(Bytes)], None)
domain, generation, authority = P(), P(), P()
assert api('mesh_authority_domain_create_v2', [C.POINTER(P)])(C.byref(domain)) == 0
assert api('mesh_authority_generation_v2', [P,C.POINTER(P)])(domain, C.byref(generation)) == 0
assert api('mesh_authority_bind_v2', [P,C.POINTER(P),U,P,C.POINTER(P)])(domain, (P*1)(generation), 1, None, C.byref(authority)) == 0
handle = P(1)
assert small(authority, b'ab', 2, C.byref(handle)) == 2 and not handle.value
for bits in [2,4,8,16,32]:
    assert api('mesh_library_enable_protected_v2', [U])(bits) == 11
assert api('mesh_library_enable_protected_v2', [U])(1) == 0
assert api('mesh_library_init', [])() == 0
for owner, data, length, expected in [(None,b'a',1,1),(authority,None,1,1),(authority,b'a',1048577,6)]:
    handle = P(1)
    assert small(owner,data,length,C.byref(handle)) == expected and not handle.value
assert small(authority,b'a',1,None) == 1
out = Bytes(1,7)
assert take(None,C.byref(out)) == 1 and not out.data and out.len == 0
assert cancel(None) == 1
release(None)
def complete(handle):
    deadline = time.monotonic()+5
    while time.monotonic() < deadline:
        output = Bytes()
        code = take(handle,C.byref(output))
        if code != 12:
            return code,output
        assert not output.data and output.len == 0
        time.sleep(.001)
    raise AssertionError('completion deadline')
for payload, expected in [(b'ab',0),(b'abc',6),(b'',0)]:
    handle = P()
    assert small(authority,payload,len(payload),C.byref(handle)) == 0
    assert take(handle,None) == 1  # No state consumption.
    code,out = complete(handle)
    assert code == expected
    result = C.string_at(out.data,out.len) if out.data else b''
    assert result == (payload if expected == 0 else b'')
    release(handle)  # Returned bytes own storage independently of handle.
    assert (C.string_at(out.data,out.len) if out.data else b'') == result
    free(C.byref(out))
payload = b'x' * 1048577
handle = P()
assert large(authority,payload,len(payload),C.byref(handle)) == 0
code,out = complete(handle)
assert code == 0 and C.string_at(out.data,out.len) == payload
assert take(handle,C.byref(Bytes())) == 13
assert cancel(handle) == 0
release(handle)
assert api('mesh_library_shutdown', [])() == 0
assert C.string_at(out.data,out.len) == payload
free(C.byref(out))
handle = P(1)
assert small(authority,b'a',1,C.byref(handle)) == 2 and not handle.value
for name, value in [('mesh_authority_release_v2',authority),('mesh_authority_generation_release_v2',generation),('mesh_authority_domain_release_v2',domain)]:
    api(name,[P],None)(value)
print(json.dumps({'bounds':'PASS','state':'PASS','post_shutdown_bytes':'PASS','unsupported_bits':[2,4,8,16,32]}))
