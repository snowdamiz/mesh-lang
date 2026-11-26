"""Retry the AMD64 lifecycle proof with a short socket path and the same native ELFs."""
from datetime import datetime, timedelta, timezone
import fcntl
import json
import subprocess
from pathlib import Path
import sys
import tarfile
from uuid import uuid4

sys.path.insert(0,'tools/durable-jobs-bench')
import run
from interleave import HOST_IDLE, validate

state=Path(__file__).resolve().parent
queue=state/'requests'
request=run.read_json(state/'controller-receipts/worker-execution-v3/request.json')
sha=request['source_sha']
base='/var/lib/durable-jobs-bench/native'
artifacts=Path(request['remote_artifacts'])
source=f'{base}/source-worker-execution-v3-{sha[:8]}'
old=request['remote_setup_evidence']
tool=f'{base}/toolchain/pgsql-17.11'
out=Path(request['evidence_destination']).with_name('native-proof-retry')
out.mkdir()
launch = HOST_IDLE + f"""
set -euo pipefail
exec 7>&1 1>&2
base={base}
proof=$(mktemp -d "$base/wpr.XXXXXX")
trap 'rm -rf -- "$proof"' EXIT
install -m 0755 {old}/compiled/trusted-effects "$proof/trusted-effects"
install -m 0755 {old}/compiled/drop-receipt.so "$proof/drop-receipt.so"
sha256sum "$proof"/{{trusted-effects,drop-receipt.so}}
chown nobody:nogroup "$proof"
mode=0
taskset -c 0,1 /usr/sbin/runuser -u nobody -- env -i PG_BIN_DIR={tool}/bin \
  PATH=/usr/bin:/bin bash {source}/scripts/fixtures/jobs-trusted-worker/run-native.sh {source} "$proof" \
  > "$proof/fault-proof.log" 2>&1 || mode=$?
printf '%s\n' "$mode" > "$proof/proof.exit"
items=(fault-proof.log proof.exit)
test ! -d "$proof/results" || items+=(results)
tar -czf - -C "$proof" "${{items[@]}}" >&7
exit "$mode"
""" 
status=1
with (run.SUITE/'.timing.lock').open('a') as lock:
    fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
    run.ssh(['bash','-c',HOST_IDLE],timeout=12).check_returncode()
    now=datetime.now(timezone.utc)
    with (queue/'STATUS.md').open('a') as log:
        log.write(f'\nSETUP worker-execution-v3 {now:%Y-%m-%dT%H:%M:%SZ} {now+timedelta(minutes=35):%Y-%m-%dT%H:%M:%SZ}\n\n'
                  f'Owner: matrix implementation operator. Retry exact {sha} lifecycle binaries with a short socket directory, then run both no-op smokes. No build.\n')
    try:
        with (out/'proof.tar.gz').open('xb') as raw,(out/'proof-driver.log').open('xb') as log:
            proof=run.ssh(['bash','-c',launch],stdout=raw,stderr=log)
        status=proof.returncode
        (out/'proof.exit').write_text(f'{status}\n')
        if status==0:
            status=1
            identity=run.ssh(['sha256sum',artifacts/'mesh',artifacts/'mesh.json'],
                             capture_output=True,text=True,timeout=20)
            identity.check_returncode()
            identity_files={name:hash for hash,name in map(str.split,identity.stdout.splitlines())}
            first=Path(request['evidence_destination'])/'setup-evidence.tar.gz'
            with tarfile.open(first,'r:gz') as raw:
                build=run.file_json(raw,'./mesh.json')
            run.require(build['version']==sha and build['files']['mesh']==identity_files[str(artifacts/'mesh')],
                        'runner changed after first build')
            rel=['tools/durable-jobs-bench/native/'+name for name in ('run','arm.py','ledger.py')]
            rel.extend(('scripts/jobs-parity.py','scripts/jobs-parity-profiles.json'))
            rel.extend('proof/jobs-parity/linux-regression/'+name for name in
                       ('trial.py','proof_transport.py','oban_runtime.py','application.sql','fence.sql'))
            bundle=Path(request['source_archive'])
            run.require(run.digest(bundle)==request['source_archive_sha256'],'source package changed')
            from hashlib import sha256
            with tarfile.open(bundle,'r:gz') as archive:
                names={member.name.removeprefix('./'):member for member in archive.getmembers()}
                for p in rel:
                    with archive.extractfile(names[p]) as data:
                        identity_files[f'{source}/{p}']=sha256(data.read()).hexdigest()
            pins=run.read_json(state/'peer-executables.json')
            seed=run.read_json(state/'current-2cpu.json')
            engines=seed['lanes']['current']['inventory']['engines']
            for engine in ('oban','go','rust'):
                engines[engine]['verified_files'].update(pins[engine])
            manifest=identity_files[str(artifacts/'mesh.json')]
            lanes={}
            for mode in ('strict','trusted-persistent'):
                label='mesh-'+mode
                spec=dict(engines['mesh'], version=sha, artifact_sha256=manifest,
                          verified_files=identity_files, argv=['/usr/bin/env',
                          f'NATIVE_JOBS_ARTIFACTS={artifacts}',f'NATIVE_JOBS_MESH_EXECUTION={mode}',
                          f'{source}/tools/durable-jobs-bench/native/run','mesh'])
                inventory=dict(cpus=[0,1],engines={**engines,'mesh':spec})
                lanes[label]=dict(engine='mesh',inventory=inventory)
                arm=out/mode
                arm.mkdir()
                remote=run.REMOTE/f'worker-proof-retry-{mode}-{uuid4().hex}'
                inv=arm/'inventory.json'
                run.write_json(inv,inventory)
                with (arm/'stage.log').open('xb') as log:
                    run.stage(run.SUITE/'worker.py',remote,log)
                    run.stage(inv,remote,log)
                block={'profile':'noop'}
                run.dispatch_arm(arm,remote,block,'diagnostic','mesh')
                folder=arm/'diagnostic-mesh'
                score=run.score_arm(folder,block,'diagnostic','mesh',inventory)
                with tarfile.open(folder/'raw.tar.gz','r:gz') as archive:
                    observed=run.file_json(archive,'adapter/mesh-execution.json')
                run.write_json(arm/'reconciled.json',dict(score,execution=observed))
            for label,engine in [('oban','oban'),('river','go'),('apalis','rust')]:
                lanes[label]=dict(engine=engine,inventory=dict(cpus=[0,1],engines=engines))
            config=dict(seed,contract='persistent-worker',lanes=lanes,description=(
                'Separately labeled supervised persistent-worker mode. Both Mesh arms use the '
                'same release artifact. The strict arm keeps per-attempt process isolation. '
                'The persistent arm reuses a worker with its own deadline watchdog and generation fence.'))
            validate(config)
            with (out/'series-config.json').open('x') as stream:
                json.dump(config,stream,indent=2)
                stream.write('\n')
            status=0
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError, tarfile.TarError) as error:
        status = status or 1
        run.write_json(out/'failure.json',dict(error=str(error),type=type(error).__name__))
        print(error,file=sys.stderr,flush=True)
    finally:
        run.ssh(['bash','-c',HOST_IDLE],timeout=12).check_returncode()
released=run.utc_now()
run.write_json(queue/'worker-execution-v3-proof-retry.released.json',
               dict(id='worker-execution-v3',released_at=released,exit_status=status,receipt=str(out)))
with (queue/'STATUS.md').open('a') as log:
    log.write(f'\nRELEASED worker-execution-v3 {released}, proof retry exit {status}. Evidence: {out}\n')
sys.exit(status)
