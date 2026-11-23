"""Freeze and smoke-test the two Mesh EXTERNAL variants for the native matrix."""
import argparse
from copy import deepcopy
import fcntl
import json
from pathlib import Path
import shlex
import subprocess
import sys
import tarfile
from uuid import uuid4

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[1]))
import run

BASE = "/var/lib/durable-jobs-bench/native"
SEED = run.RESULTS / "2026-11-23-direct-producer-v3-setup/large-executable-pins.json"
LABELS = {"mesh-external": "mesh-stock-a", "mesh-direct-external": "mesh-direct-file"}

def plan(config):
    rows = []
    for label, original in LABELS.items():
        arm = config["lanes"][original]["inventory"]["engines"]["mesh"]
        source = Path(arm["argv"][-2]).parent
        files = {
            Path(p).name: digest for p,digest in arm["verified_files"].items()
            if Path(p).parent == source
        }
        expected = {"arm.py", "run", "ledger.py"}
        if original == "mesh-direct-file":
            expected.add("direct-trial.py")
        if set(files) != expected:
            raise ValueError(f"incomplete pinned input adapter {files.keys()}")
        rows.append(dict(label=label, source=str(source), output=str(source.parent/f"{label}-storage-v1"),files=files))
    return {"lanes":rows}

def apply_pins(seed, rows, evidence):
    lanes = deepcopy(seed["lanes"])
    for row in rows:
        label = row["label"]
        updated = deepcopy(lanes[LABELS[label]])
        engine = updated["inventory"]["engines"]["mesh"]
        engine["argv"][-2] = f'{row["output"]}/run'
        for fname in row["files"]:
            path = f'{row["output"]}/{fname}'
            engine["verified_files"][path] = evidence[path]
        run.check_inventory(updated["inventory"])
        lanes[label] = updated
    selected = ["mesh-stock-a","mesh-external","mesh-direct-external","oban","river","apalis"]
    out = deepcopy(seed)
    out["description"] = ("Protected, fresh-exec Mesh. Compare default stock broker, external storage with stock broker, "
        "and external storage with direct-file broker against the three stock peers. All use fresh databases. "
        "The catalog and stored payloads are checked outside the submit window.")
    out["lanes"] = {label:lanes[label] for label in selected}
    cross = deepcopy(out)
    cross["description"] = "Crossed Mesh stock/direct producer and default/external payload storage, identical worker."
    cross["lanes"] = {label:lanes[label] for label in ("mesh-stock-a","mesh-direct-file","mesh-external","mesh-direct-external")}
    return out,cross

def read_storage(archive):
    with tarfile.open(archive,"r:gz") as files:
        def read(name):
            hits = [m for m in files if m.isfile() and Path(m.name).name == name]
            if len(hits) != 1:
                raise ValueError(f"{archive} missing one {name}")
            return json.load(files.extractfile(hits[0]))
        first,rows = read("payload-storage-before.json"),read("payload-storage-after.json")
        jobs = read("ledger.json")["jobs"]
    population = {(j["id"],j["payload_sha256"]) for j in jobs}
    checks = (
        first["before"]["attstorage"] == "x", first["after"]["attstorage"] == "e",
        first["before"]["rows"] == first["after"]["rows"] == 0,
        len(rows) == len(population) == 64,
        {(j["id"],j["payload_sha256"]) for j in rows} == population,
        all(r["compression"] is None and r["state"] == "completed" and r["attempt"] == 1 for r in rows),
    )
    if not all(checks):
        raise ValueError(f"storage policy or ledger mismatch {checks}")
    return {"jobs":len(rows),"payload_digests":len(population),"stored_bytes":sum(r["stored_bytes"] for r in rows),
            "ddl_client_ns":first["ddl_client_ns"],"raw_sha256":run.digest(archive)}

def phase(out):
    out.mkdir(parents=True,exist_ok=True)
    seed = run.read_json(SEED)
    config = plan(seed)
    run.write_json(out/"request.json",config)
    installer = HERE/"prepare.py"
    source_hash = run.digest(installer)
    # Task-owned immutable variants. The two underlying matrix adapters are read-only inputs.
    command = f"""
set -eu
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<{BASE}/native.lock
flock -n 9
umask 022
file=$(mktemp {BASE}/tmp/storage-v1.XXXXXX)
trap 'rm -f -- "$file"' EXIT
cat > "$file"
printf '%s  %s\\n' {source_hash} "$file" | sha256sum -c >&2
python3 -B "$file" {shlex.quote(json.dumps(config))}
"""
    with (run.SUITE/".timing.lock").open("a") as lock:
        fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        with installer.open("rb") as src,(out/"install.log").open("xb") as log:
            result = run.ssh(["bash","-c",command],stdin=src,stdout=subprocess.PIPE,stderr=log,timeout=35)
            result.check_returncode()
        pins = json.loads(result.stdout)
        run.write_json(out/"native-pins.json",pins)
        full,cross = apply_pins(seed,config["lanes"],pins)
        run.write_json(out/"large.json",full)
        run.write_json(out/"crossed.json",cross)
        for label in LABELS:
            local = out/label
            local.mkdir()
            inventory = full["lanes"][label]["inventory"]
            run.write_json(local/"inventory.json",inventory)
            remote = run.REMOTE/f"payload-storage-{label}-{uuid4().hex}"
            with (local/"stage.log").open("xb") as log:
                run.stage(run.SUITE/"worker.py",remote,log)
                run.stage(local/"inventory.json",remote,log)
            block={"profile":"large"}
            run.dispatch_arm(local,remote,block,"diagnostic","mesh")
            arm=local/"diagnostic-mesh"
            score=run.score_arm(arm,block,"diagnostic","mesh",inventory)
            proof=read_storage(arm/"raw.tar.gz")
            run.write_json(local/"reconciled.json",dict(score=score,storage=proof))
    run.write_json(out/"stage-complete.json",dict(installer_sha256=source_hash,seed_sha256=run.digest(SEED),
        large_sha256=run.digest(out/"large.json"),crossed_sha256=run.digest(out/"crossed.json")))

if __name__ == "__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output",type=Path)
    args=parser.parse_args()
    phase(args.output)
