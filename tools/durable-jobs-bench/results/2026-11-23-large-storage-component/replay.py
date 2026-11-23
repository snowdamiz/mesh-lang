#!/usr/bin/env python3
"""Verify the original storage experiment and optionally reproduce it with PG 17.11 on macOS."""
import argparse
import hashlib
import json
import os
from pathlib import Path
from statistics import median
import subprocess
import sys
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
def checksum(data):
    return hashlib.sha256(data).hexdigest()

def files():
    manifest = json.loads((HERE/"manifest.json").read_text())
    with tarfile.open(HERE/"evidence.tar.gz","r:gz") as archive:
        members = archive.getmembers()
        if len(members) != len(manifest["files"]) or not all(x.isfile() for x in members):
            raise ValueError("unexpected archive members")
        blobs = {m.name:archive.extractfile(m).read() for m in members}
    if {k:checksum(v) for k,v in blobs.items()} != manifest["files"]:
        raise ValueError("captured source or evidence changed")
    return blobs,manifest

def reduce(blobs):
    report = {}
    for workload in ("frozen","compressible"):
        def load(file):
            return json.loads(blobs[f"{workload}/{file}"])
        manifest = load("inputs.json")
        if manifest.get("seed_input_sha256",manifest.get("input_sha256")) != "9d7f130f2cc690487582f73c9bea5fbb1f2661f8e416beb2f1574a6f868cc5c1":
            raise ValueError("different frozen input seed")
        arms = {}
        blocks = []
        for i,order in enumerate(manifest["orders"]):
            if set(order) != {"control_a","control_b","external","lz4"}:
                raise ValueError("incorrect block")
            block = {}
            for arm in order:
                row = load(f"{i}-{arm}.json")
                if row["terminal"] != [["completed","1","64"]] or sum(int(count) for _,count,_ in row["storage"]) != 64:
                    raise ValueError("incorrect terminal or storage counts")
                if workload == "frozen" and row["storage"] != [["none","64","33559808"]]:
                    raise ValueError("frozen payload storage changed")
                block[arm] = row
                arms.setdefault(arm,[]).append(row)
            blocks.append(block)
        report[workload] = {"groups":len(blocks),"arms":{},"paired":{}}
        for arm,observations in arms.items():
            if len(observations) != 4:
                raise ValueError("incomplete series")
            metrics = {
                f"{stage}_{clock}":median(x[stage][clock] for x in observations)
                for stage in ("insert","claims","settlements")
                for clock in ("backend_cpu_ms","wall_ms")
            }
            metrics["stored_bytes"] = median(sum(int(n) for _,_,n in x["storage"]) for x in observations)
            report[workload]["arms"][arm] = metrics
        for arm in ("external","lz4"):
            report[workload]["paired"][arm] = {
                clock: [min(b["control_a"]["insert"][clock],b["control_b"]["insert"][clock])-b[arm]["insert"][clock] for b in blocks]
                for clock in ("backend_cpu_ms","wall_ms")
            }
    return report

def fresh(blobs,manifest,dest):
    if sys.platform!="darwin":
        raise ValueError("the original component uses macOS process CPU accounting")
    if dest.exists():
        raise ValueError("select a fresh evidence destination")
    if not dest.is_relative_to(ROOT) or dest == ROOT:
        raise ValueError("the result directory must be inside the checkout")
    versions = {
        "scripts/jobs-parity.py":"target/large-claim-perf/frozen/jobs-parity.py",
        "scripts/jobs-parity-profiles.json":"target/large-claim-perf/frozen/jobs-parity-profiles.json",
        "compiler/mesh-rt/src/jobs/schema.sql":"compiler/mesh-rt/src/jobs/schema.sql",
    }
    for path in versions:
        if checksum((ROOT/path).read_bytes()) != manifest["repo_inputs"][path]:
            raise ValueError(f"pinned input changed: {path}")
    dest.mkdir(parents=True)
    # A short path keeps the Unix socket address inside the macOS size limit.
    with tempfile.TemporaryDirectory(prefix=".sq-", dir=ROOT) as temp:
        work=Path(temp)
        for source,local in versions.items():
            if not local.startswith("target/"):
                continue
            path=work/local.removeprefix("target/")
            path.parent.mkdir(parents=True,exist_ok=True)
            path.write_bytes((ROOT/source).read_bytes())
        for member,data in blobs.items():
            if member.startswith("support/target/"):
                path=work/member.removeprefix("support/target/")
                path.parent.mkdir(parents=True,exist_ok=True)
                path.write_bytes(data)
        lock=ROOT/"target/queue-performance.lock"
        lock.parent.mkdir(exist_ok=True)
        lock.touch(exist_ok=True)
        (work/"queue-performance.lock").symlink_to(lock)
        lane=work/"native-large-payload-storage"
        lane.mkdir()
        for kind,flags in (("frozen",[]),("compressible",["compressible"])):
            (lane/"screen.py").write_bytes(blobs[f"{kind}/screen.py"])
            with (dest/f"{kind}.log").open("xb") as log:
                result=subprocess.run([sys.executable,"-B",str(lane/"screen.py"),kind,*flags],
                                      stdout=log,stderr=subprocess.STDOUT,env=os.environ)
            if (lane/kind).exists():
                (lane/kind).rename(dest/kind)
            result.check_returncode()

if __name__ == "__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run",type=Path)
    args=parser.parse_args()
    blobs,manifest=files()
    summary=reduce(blobs)
    if summary != json.loads((HERE/"summary.json").read_text()):
        raise ValueError("saved medians or matched differences changed")
    if args.run:
        fresh(blobs,manifest,args.run.resolve())
    print(json.dumps(summary,indent=2,sort_keys=True))
