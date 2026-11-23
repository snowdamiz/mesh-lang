"""Dispatch admitted Linode SETUP and seal separate short, noop and ordinary-V1 slow inventories."""
import copy
from datetime import datetime, timezone
import fcntl
import hashlib
from pathlib import Path
import re
import subprocess
import sys
import tarfile
from uuid import uuid4

REPLAY=Path(__file__).resolve().parent
LANE=REPLAY.parent
ROOT=LANE.parents[1]
SUITE=ROOT/"tools/durable-jobs-bench"
sys.path.insert(0,str(SUITE))
import run
from verify import reconcile

REQUESTS=ROOT/"target/linode-jobs-stepchange/requests"
NAME="bulk-enqueue"
REMOTE="/var/lib/durable-jobs-bench/native"
HOST_FILES=(
    "scripts/jobs-parity.py", "scripts/jobs-parity-profiles.json",
    "proof/jobs-parity/linux-regression/trial.py",
    "proof/jobs-parity/linux-regression/proof_transport.py",
    "proof/jobs-parity/linux-regression/application.sql",
    "proof/jobs-parity/linux-regression/oban_runtime.py",
    "proof/jobs-parity/linux-regression/fence.sql",
    "proof/jobs-parity/linux-regression/trusted-application.sql",
    "tools/durable-jobs-bench/native/run", "tools/durable-jobs-bench/native/arm.py",
    "tools/durable-jobs-bench/native/ledger.py",
)
# The V1 slow inventory keeps every lane of the first native ordinary V1 slow
# rotation byte-for-byte and adds the set-based candidate (and its scalar switch).
V1_FOLLOWUP=ROOT/"target/ordinary-trusted-mesh-queue/evidence/native/qualified-followup.json"
V1_FOLLOWUP_SHA256="805bac3d3521f2b663d10faa5e9b250eff8f64fe7c04deb60cddcddb2f674439"
PROFILES={"source":("short","noop"),"ordinary-v1":("short","noop","slow"),"ordinary-v2":("short","noop")}
KINDS={"source":"strict-frozen","ordinary-v1":"ordinary-trusted-shared-process",
       "ordinary-v2":"ordinary-trusted-shared-process-v2"}

def confirm_grant():
    events=re.findall(r"^(SETUP|TIMING|RELEASED) ([\w-]+) (\S+)(?: (\S+))?",
                      (REQUESTS/"STATUS.md").read_text(),re.MULTILINE)
    kind,request,start,end=events[-1]
    now=datetime.now(timezone.utc)
    run.require((kind,request)==("SETUP",NAME) and end and
                datetime.fromisoformat(start)<=now<datetime.fromisoformat(end),
                "controller's exclusive SETUP grant is required")
    return start,end

def seal(out, bundle, attempt, binding):
    source=f"{REMOTE}/source-{attempt}"
    proof_source={lane:f"{source}/{lane}" for lane in KINDS}
    build={lane:f"{REMOTE}/builds/{attempt}-{lane}" for lane in KINDS}
    reference=run.read_json(REPLAY/"peer-inventory.json")
    with tarfile.open(bundle) as packaged, tarfile.open(out/"raw.tar.gz") as raw:
        raw.getmember("./wire-and-mixed/PASS")
        def item(path):
            return run.file_json(raw,"./"+path)

        lines=raw.extractfile("./setup-identity.txt").read().decode().splitlines()
        machine,boot,disk,ticks=map(str.strip,lines[:4])
        host=[boot,machine.split()[1],disk]
        run.require(machine.startswith("Linux ") and machine.endswith(" x86_64") and host==reference["host"],
                    "host changed from the pinned stock-peer inventory")
        run.require(int(ticks)==reference["clock_ticks"],"host tick rate drift")
        artifacts={name:digest for digest,name in [
            line.split() for line in raw.extractfile("./artifacts.txt").read().decode().splitlines()]}
        for lane,contract in KINDS.items():
            for profile in PROFILES[lane]:
                tag=f"{lane}-{profile}"
                ledger=item(f"{tag}/ledger.json")
                identity=item(f"{tag}/build.json")
                writer=item(f"{tag}/producer-writer.json")
                row=item(f"{tag}/row.json")
                run.require(writer["enabled"] and writer["bulk"] > 0,"smoke selected the scalar writer")
                run.require(identity["version"]==binding["source_sha"] and row["artifact_sha256"]==artifacts[f"{build[lane]}/mesh.json"]
                            and identity["files"]["mesh"]==artifacts[f"{build[lane]}/mesh"],"smoke build identity drift")
                run.require(all(mask==[0,1] for mask in item(f"{tag}/process-affinity.json").values()),
                            "native setup used a different CPU mask")
                if lane!="source":
                    run.require(ledger["execution_contract"]==identity["execution_contract"]==contract,
                                "wrong ordinary execution contract")
                if lane=="ordinary-v2":
                    policy="success_cohort" if profile=="short" else "stock"
                    run.require(ledger["queue_policy"]==policy, "wrong v2 queue policy")
                report=reconcile(ledger,"mesh",profile,{"version":identity["version"],
                                  "artifact_sha256":row["artifact_sha256"]})
                run.write_json(out/f"{tag}-reconciled.json",report)

                stock_reference=reference["lanes"]["current"]
                peer_lanes={tag:copy.deepcopy(reference["lanes"][tag]) for tag in ("river","oban","apalis")}
                if profile=="slow":
                    run.require(run.digest(V1_FOLLOWUP)==V1_FOLLOWUP_SHA256,"V1 slow follow-up config changed")
                    followup=run.read_json(V1_FOLLOWUP)
                    run.require(followup["host"]==host and followup["clock_ticks"]==int(ticks),
                                "host changed from the first native V1 slow rotation")
                    stock_reference=followup["lanes"]["ordinary"]
                    peer_lanes={tag:copy.deepcopy(lane_) for tag,lane_ in followup["lanes"].items()}
                candidates={}
                for enabled,label in ((False,"scalar"),(True,"bulk")):
                    if lane=="source" and not enabled:
                        candidates[label]=copy.deepcopy(stock_reference)
                        continue
                    bench=copy.deepcopy(stock_reference)
                    arm=bench["inventory"]["engines"]["mesh"]
                    files={f"{build[lane]}/{name}":artifacts[f"{build[lane]}/{name}"] for name in ("mesh","mesh.json")}
                    for name in HOST_FILES:
                        try:
                            member=packaged.extractfile(f"{lane}/{name}")
                        except KeyError:
                            continue
                        with member:
                            files[f"{proof_source[lane]}/{name}"]=hashlib.file_digest(member,"sha256").hexdigest()
                    extra=[]
                    if lane=="ordinary-v2":
                        extra=[f"TRUSTED_ADMISSION_POLICY={policy}"]
                    arm.update(argv=["/usr/bin/env",*extra,f"MESH_JOBS_SET_BASED_ENQUEUE={int(enabled)}",
                                     f"NATIVE_JOBS_ARTIFACTS={build[lane]}",
                                     f"{proof_source[lane]}/tools/durable-jobs-bench/native/run","mesh"],
                               version=binding["source_sha"],artifact_sha256=artifacts[f"{build[lane]}/mesh.json"],
                               verified_files=files)
                    arm["evidence_globs"]=[*arm["evidence_globs"],"producer-writer.json"]
                    if lane!="source":
                        bench["execution_contract"]=contract
                    if lane=="ordinary-v2":
                        bench["queue_policy"]=policy
                    candidates[label]=bench
                config=dict(contract=contract,profiles=[profile],cpus=[0,1],host=host,clock_ticks=int(ticks),
                            description=f"Matched {contract} scalar and set-based producer on frozen {profile} inputs.",
                            lanes={**candidates,**peer_lanes})
                if profile=="slow":
                    config.update({k:v for k,v in followup.items() if k not in ("lanes","profiles","description")},
                                  profiles=["slow"],
                                  description="First native ordinary V1 slow rotation (original V1, protected stock, "
                                              "River, Oban, Apalis unchanged) plus the set-based V1 candidate and its "
                                              "scalar switch on one executable.",
                                  lanes={**candidates,**peer_lanes})
                run.write_json(out/f"{lane}-{profile}.json",config)

def main():
    binding=run.read_json(REQUESTS/f"{NAME}.json")
    run.require(binding["state"]=="ready" and binding["ready"],"request is not READY")
    for name,digest in binding["pins"].items():
        run.require(run.digest(Path(name))==digest,f"input digest changed: {name}")
    bundle=Path(binding["source_archive"])
    attempt=f"bulk-enqueue-{uuid4().hex[:12]}"
    out=LANE/"evidence/native"/attempt
    with (SUITE/".timing.lock").open("r") as lock:
        fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        start,end=confirm_grant()
        out.mkdir(parents=True)
        run.write_json(out/"grant.json",dict(attempt=attempt,start=start,end=end,
            source_sha=binding["source_sha"],source_archive_sha256=binding["source_archive_sha256"]))
        code=None
        try:
            with bundle.open("rb") as stdin,(out/"setup.log").open("xb") as log:
                script=(REPLAY/"native-setup.sh").read_text()
                budget=(datetime.fromisoformat(end)-datetime.now(timezone.utc)).total_seconds()-90
                run.require(budget>=120,"insufficient time to execute and archive the slot")
                result=run.ssh(["bash","-c",script,"--",binding["source_archive_sha256"],attempt,binding["source_sha"]],
                               stdin=stdin,stdout=log,stderr=log,timeout=min(1800,budget))
                code=result.returncode
        finally:
            run.write_json(out/"status.json",dict(exit_code=code,utc=run.utc_now()))
            with (out/"raw.tar.gz").open("xb") as stream,(out/"export.log").open("xb") as log:
                export=run.ssh(["tar","-czf","-","-C",f"{REMOTE}/runs/{attempt}","."],
                               stdout=stream,stderr=log,timeout=90)
                export.check_returncode()
        result.check_returncode()
        seal(out,bundle,attempt,binding)
    print(out)

if __name__=="__main__":
    main()
