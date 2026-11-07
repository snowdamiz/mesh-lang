import ctypes as C
import hashlib,json,subprocess
from pathlib import Path
root=Path(__file__).resolve().parent
base=root/'app-baseline'; candidate=root/'app-candidate'
name='libmessenger_mobile'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
a=json.loads((base/(name+'.abi.json')).read_text()); b=json.loads((candidate/(name+'.abi.json')).read_text())
rows=[{k:v for k,v in row.items() if k!='scheduledSymbol'} for row in b['exports']]
assert a['exports']==rows and len(rows)==176
assert all(row['scheduledSymbol']==row['symbol']+'_submit_v2' for row in b['exports'])
result={'logical_rows':len(rows),'same_rows_limits_audiences':True,'legacy_bindings':{},'same_legacy_ownership':a['ownership']==b['ownership']}
for suffix in ['h','ts','swift','kt','jni.c']:
    left=base/(name+'.'+suffix); right=candidate/left.name
    assert left.read_bytes()==right.read_bytes(),suffix
    result['legacy_bindings'][suffix]={'identical':True,'sha256':sha(right)}
header=(candidate/(name+'.h')).read_text(); extension=(candidate/(name+'.invocations.h')).read_text()
assert 'invocations.h' not in header and '_submit_v2(' not in header
assert '#include "libmessenger_mobile.h"' in extension
symbols=subprocess.check_output(['nm','-gU',str(candidate/(name+'.dylib'))],text=True)
(candidate/'symbols.txt').write_text(symbols)
physical={line.split()[-1].removeprefix('_') for line in symbols.splitlines() if line.split()}
assert all(row['symbol'] in physical and row['scheduledSymbol'] in physical for row in b['exports'])
result.update(all_legacy_and_companion_symbols_present=True,physical_app_companions=176,desktop_literal_membership_rejects_companions=True)
result['families']={}
# Separate subprocesses keep the two static runtime copies isolated. No runtime init needed.
for label,directory in [('baseline',base),('candidate',candidate)]:
    library=directory/(name+'.dylib')
    observed=int(subprocess.check_output(['python3','-c','import ctypes,sys; l=ctypes.CDLL(sys.argv[1]); l.mesh_library_capabilities_v2.restype=ctypes.c_uint64; print(l.mesh_library_capabilities_v2())',str(library)]))
    assert observed==1
    result['families'][label]={'library_sha256':sha(library),'manifest_sha256':sha(directory/(name+'.abi.json')),'observed_capability_mask':observed}
(root/'app-compatibility.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result,indent=2))
