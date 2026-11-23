"""Replay the set-based enqueue component; optionally repeat it in a new local PG cluster."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
from statistics import median
import subprocess
import sys
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent / "2026-11-23-large-storage-component"
ROOT = HERE.parents[3]
def read_capture():
    manifest = json.loads((HERE / "manifest.json").read_text())
    with tarfile.open(HERE / "evidence.tar.gz", "r:gz") as archive:
        members = archive.getmembers()
        if len(members) != len(manifest) or any(not m.isfile() for m in members):
            raise ValueError("unexpected archive entries")
        blobs = {m.name: archive.extractfile(m).read() for m in members}
    if {k:hashlib.sha256(v).hexdigest() for k,v in blobs.items()} != manifest:
        raise ValueError("receipt or original source changed")
    return blobs

def summarize(blobs):
    load = lambda name: json.loads(blobs[name])
    header = load("manifest.json")
    if hashlib.sha256(blobs["bulk.sql"]).hexdigest() != header["sql_sha256"]:
        raise ValueError("SQL differs from the executed probe")
    stats = {}
    for profile, count in (("short",128),("noop",1024)):
        cases = {}
        blocks = []
        for i, order in enumerate(header["orders"]):
            if set(order) != {"stock_a","ordered","bulk","stock_b"} or len(order) != 4:
                raise ValueError("different arm order")
            block = {}
            for arm in order:
                row = load(f"{i}-{profile}-{arm}.json")
                if row["rows"] != count or min(row["wall_ms"],row["backend_cpu_ms"],row["driver_cpu_ms"]) <= 0:
                    raise ValueError("invalid successful component receipt")
                cases.setdefault(arm,[]).append(row)
                block[arm] = row
            blocks.append(block)
        stats[profile] = dict(jobs_per_arm=count, groups=len(blocks),
            medians={arm:{key:median(r[key] for r in rows)
                          for key in ("backend_cpu_ms","driver_cpu_ms","wall_ms")}
                     for arm,rows in cases.items()},
            paired_minimum_saving_ms={
                arm:{key:[min(b["stock_a"][key],b["stock_b"][key])-b[arm][key] for b in blocks]
                     for key in ("backend_cpu_ms","wall_ms")} for arm in ("bulk","ordered")})
    return stats

def rerun(blobs,dest):
    if dest.exists() or dest == ROOT or not dest.is_relative_to(ROOT) or sys.platform != "darwin":
        raise ValueError("choose a new in-checkout evidence path on macOS")
    support = importlib.util.spec_from_file_location("storage_replay",PARENT/"replay.py")
    storage = importlib.util.module_from_spec(support)
    support.loader.exec_module(storage)
    base, manifest = storage.files()
    for name, required in manifest["repo_inputs"].items():
        if hashlib.sha256((ROOT/name).read_bytes()).hexdigest() != required:
            raise ValueError(f"pinned repository input changed: {name}")
    dest.parent.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".be-",dir=ROOT) as temp:
        work=Path(temp)
        for name,data in base.items():
            if name.startswith("support/target/"):
                path=work/name.removeprefix("support/target/")
                path.parent.mkdir(parents=True,exist_ok=True)
                path.write_bytes(data)
        shared=work/"native-large-payload-storage"
        shared.mkdir()
        (shared/"screen.py").write_bytes(base["compressible/screen.py"])
        lane=work/"native-bulk-job-enqueue"
        lane.mkdir()
        for name in ("bulk.sql","screen.py"):
            (lane/name).write_bytes(blobs[name])
        lock=ROOT/"target/queue-performance.lock"
        lock.parent.mkdir(exist_ok=True)
        lock.touch(exist_ok=True)
        (work/"queue-performance.lock").symlink_to(lock)
        result=subprocess.run([sys.executable,"-B",str(lane/"screen.py"),"out"])
        if (lane/"out").exists():
            (lane/"out").rename(dest)
        result.check_returncode()

if __name__ == "__main__":
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--run",type=Path)
    args=p.parse_args()
    raw=read_capture()
    summary=summarize(raw)
    if summary != json.loads((HERE/"summary.json").read_text()):
        raise ValueError("stored result differs from raw evidence")
    if args.run:
        rerun(raw,args.run.resolve())
    print(json.dumps(summary,indent=2,sort_keys=True))
