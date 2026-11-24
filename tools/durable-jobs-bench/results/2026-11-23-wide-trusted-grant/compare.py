"""Use one rebuilt runner for both SQL widths, with an additional original V1 control."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "2026-11-23-ordinary-claim-width"))
import screen
from screen import topology

AREA = screen.ROOT / "target/wide-trusted-grant"
LABELS = ("original", "width16_a", "width32", "width16_b")
ORDERS = [[LABELS[i] for i in sequence] for sequence in
          ((0,2,1,3), (3,1,2,0), (2,3,0,1), (1,0,3,2))]
CAP32 = screen.BEFORE.replace("16", "32")


def prepare(cluster, database, label):
    if cluster.broker or not cluster.owned(database):
        raise RuntimeError("expected fixture-owned database before producer startup")
    query = "SELECT pg_get_functiondef(" + screen.IDENTITY + ")"
    source = cluster.sql(database, query)
    assert source.count(screen.BEFORE) == 1
    expected = source.replace(screen.BEFORE, CAP32) if label.startswith("width32") else source
    if label == "width32_proof":
        for old, new in (
            ("oversized_count integer := 0;", "oversized_count integer := 0; completed_rows integer := 0;"),
            ("remaining := remaining - 1;", "completed_rows := completed_rows + 1; remaining := remaining - 1;"),
            ("CLOSE candidates;", "RAISE LOG 'widthproof count=%', completed_rows; CLOSE candidates;"),
        ):
            assert expected.count(old) == 1
            expected = expected.replace(old, new)
    if expected != source:
        cluster.sql(database, expected)
    installed = cluster.sql(database, query)
    assert expected == installed
    return {k:hashlib.sha256(v.encode()).hexdigest() for k,v in (("stock_source",source),("installed_source",installed))}


def dispatch(build, out, proof):
    built = topology.read(build / "built.json")
    original = screen.baseline.check_pins(screen.SOURCE, screen.BINS)
    bins = build / "artifacts"
    manifest = topology.read(bins / "mesh.json")
    assert built["manifest_sha256"] == topology.sha(bins/"mesh.json")
    assert built["executable_sha256"] == topology.sha(bins/"mesh")
    stock = topology.read(screen.BINS / "mesh.json")
    assert {k:v for k,v in manifest["files"].items() if k != "mesh"} == {
        k:v for k,v in stock["files"].items() if k != "mesh"}
    assert manifest["experiment"]["source_archive_sha256"] == original["source_archive_sha256"]
    out.mkdir()
    state = out / "state"
    state.mkdir()
    labels = [["width32_proof"]] if proof else ORDERS
    env = {k:v for k,v in os.environ.items()
           if not k.startswith(("MESH_", "JOBS_PARITY_", "NATIVE_JOBS_", "DURABLE_JOBS_", "PG"))}
    env.update(NATIVE_JOBS_STATE=str(state), TMPDIR=str(state), PG_BIN_DIR="/opt/homebrew/opt/postgresql@17/bin",
               DURABLE_JOBS_PHASE="diagnostic", PYTHONDONTWRITEBYTECODE="1")
    with (screen.ROOT/"target/queue-performance.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        topology.write(out/"series.json", dict(orders=labels, input=topology.INPUT, stock=original,
                       built=built, layout="tuned-2x32", pg_sha256=topology.sha(Path(env["PG_BIN_DIR"])/"postgres"),
                       compare_sha256=topology.sha(Path(__file__))))
        for number, order in enumerate(labels,1):
            folder = out/f"{number:02}"
            folder.mkdir()
            for label in order:
                arm = folder/label
                env["NATIVE_JOBS_ARTIFACTS"] = str(screen.BINS if label == "original" else bins)
                start, load = time.monotonic(), os.getloadavg()
                with arm.with_suffix(".log").open("xb") as log:
                    rc = subprocess.run([sys.executable, "-B", __file__, str(build), str(arm), "--arm",label],
                                        env=env, stdout=log, stderr=subprocess.STDOUT)
                topology.write(arm.with_suffix(".status.json"),dict(exit_code=rc.returncode,load_before=load,
                               load_after=os.getloadavg(),wall_s=time.monotonic()-start))
                print(number, label, rc.returncode,flush=True)
                rc.check_returncode()
                if proof:
                    counts = list(map(int, re.findall(r"widthproof count=(\d+)",(arm/"postgres.log").read_text())))
                    assert sum(counts) == 1024 and 16 < max(counts) <= 32
                    topology.write(out/"width-proof.json",dict(confirmed_by_completed_jobs=1024, grant_rows=counts,
                                   max_observed=max(counts),grants_over_16=sum(n>16 for n in counts)))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("build",type=Path)
    parser.add_argument("out",type=Path)
    parser.add_argument("--proof",action="store_true")
    parser.add_argument("--arm",choices=(*LABELS,"width32_proof"))
    args=parser.parse_args()
    out=args.out.resolve()
    if out == AREA or not out.is_relative_to(AREA) or out.exists():
        raise ValueError("use a new directory under target/wide-trusted-grant")
    if args.arm:
        screen.prepare = prepare
        screen.LAYOUT = {args.arm:"tuned-2x32"}
        screen.launch(args.arm,out)
    else:
        dispatch(args.build.resolve(),out,args.proof)
