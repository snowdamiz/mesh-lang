import concurrent.futures, hashlib, json, os, pathlib, subprocess, time
root=pathlib.Path.cwd(); owned=root/'.tmp/native-owned-invocations'; out=owned/'r1-baseline';out.mkdir(exist_ok=True)
family=root/'.tmp/native-effect-authority/generated-abi-1794012766838904000'
cases=['control','write','query-write','schema','commit','raw-commit','release','accepted-commit','http','store','accepted-store','nonce','accepted-nonce','actor','timer','missing','error','unsupported','generations','registration','hot-journal','terminate','v1-6','v1-8','v1-output','v1-missing','ancestry','gc','panic','request-refusal','stream-refusal']
def one(case):
 cmd=['python3','compiler/meshc/tests/fixtures/library_authority/check.py',str(family/'libauthority.dylib'),case,'--stack-probe='+str(family/'stack-probe.so')]
 p=subprocess.run(cmd,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=40);log=out/(case+'.log');log.write_bytes(p.stdout)
 return {'case':case,'command':cmd,'exit':p.returncode,'log_sha256':hashlib.sha256(p.stdout).hexdigest()}
with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool: result=list(pool.map(one,cases))
(out/'receipt.json').write_text(json.dumps({'results':result,'files':{str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in [family/'libauthority.dylib',family/'stack-probe.so',root/'compiler/meshc/tests/fixtures/library_authority/check.py']}},indent=2)+'\n')
print(json.dumps({r['case']:r['exit'] for r in result}));assert all(r['exit']==0 for r in result)
