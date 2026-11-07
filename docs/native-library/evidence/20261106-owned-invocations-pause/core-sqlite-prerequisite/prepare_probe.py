"""Direct embedded SQLite study; does not enable any Mesh scoped operation."""
import ctypes as C
import hashlib,json
from pathlib import Path
import sys
library=Path(sys.argv[1]).resolve()
lib=C.CDLL(str(library)); P=C.c_void_p; I=C.c_int

def api(name,args,result=I):
    f=getattr(lib,name); f.argtypes=args; f.restype=result; return f
open_db=api('sqlite3_open',[C.c_char_p,C.POINTER(P)])
prepare=api('sqlite3_prepare_v2',[P,C.c_char_p,I,C.POINTER(P),C.POINTER(C.c_char_p)])
step=api('sqlite3_step',[P]); finalize=api('sqlite3_finalize',[P]); close=api('sqlite3_close',[P])
integer=api('sqlite3_column_int',[P,I])
Auth=C.CFUNCTYPE(I,P,I,C.c_char_p,C.c_char_p,C.c_char_p,C.c_char_p)
set_auth=api('sqlite3_set_authorizer',[P,Auth,P])
version=api('sqlite3_libversion_number',[])()
db=P(); assert open_db(b':memory:',C.byref(db))==0

def statement(sql,run):
    stmt=P(); tail=C.c_char_p()
    rc=prepare(db,sql.encode(),-1,C.byref(stmt),C.byref(tail))
    if rc:
        assert not stmt.value
        return rc,None
    value=None
    if run:
        status=step(stmt); assert status in [100,101],status
        if status==100: value=integer(stmt,0)
    assert finalize(stmt)==0
    return rc,value

def read(name):
    return statement('PRAGMA '+name,True)[1]
defaults={name:read(name) for name in ['busy_timeout','secure_delete']}
rows=[]
for name,value in [('busy_timeout','1000'),('secure_delete','ON')]:
    assert statement('PRAGMA '+name+'=0',False)[0]==0
    before=read(name)
    assert statement('PRAGMA '+name+'='+value,False)[0]==0
    after_prepare_only=read(name)
    assert (before,after_prepare_only)==(0,1000 if name=='busy_timeout' else 1)
    assert statement('PRAGMA '+name+'=0',False)[0]==0
    seen=[]
    @Auth
    def deny(_ctx,action,one,two,_db,_source):
        if action==19:
            seen.append([one.decode() if one else None,two.decode() if two else None])
            return 1
        return 0
    assert set_auth(db,deny,None)==0
    denied=statement('PRAGMA '+name+'='+value,False)[0]
    assert denied==23
    assert set_auth(db,Auth(),None)==0
    after_refusal=read(name)
    assert after_refusal==0
    rows.append({'pragma':name,'before':before,'prepare_without_step':after_prepare_only,'authorizer_denied_status':denied,'after_denied_prepare':after_refusal,'authorizer_arguments':seen})
assert close(db)==0
reopened=P(); database=Path(__file__).with_name('reopen.db').resolve()
assert open_db(str(database).encode(),C.byref(reopened))==0
db=reopened
assert statement('CREATE TABLE IF NOT EXISTS setting_probe(value INTEGER)',True)[0]==0
assert statement('PRAGMA secure_delete='+str(1-defaults['secure_delete']),False)[0]==0
assert close(db)==0
assert open_db(str(database).encode(),C.byref(db))==0
reopen_setting=read('secure_delete')
assert reopen_setting==defaults['secure_delete']
assert close(db)==0
result={'build_defaults':defaults,'reopened_secure_delete':reopen_setting,'scope':'direct embedded SQLite only; Mesh policy unchanged','sqlite_version_number':version,'library':str(library),'library_sha256':hashlib.sha256(library.read_bytes()).hexdigest(),'cases':rows}
Path(__file__).with_name('prepare-probe.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result,indent=2))
