"""Replay the component price and the live helper compatibility probe."""
import argparse
import gzip
import io
import json
from pathlib import Path
import runpy
from statistics import median
import tarfile

from screen import ARMS, BASE, HERE, ORDERS, ROOT, SUPPORT, fixture, schema_for

METRICS = runpy.run_path(str(HERE.parent/"2026-11-23-large-arg-render/replay.py"))["metrics"]
ARCHIVE = HERE / "receipts.tar.gz"


def seal():
    scratch = ROOT / "target/enqueue-receipt-projection"
    paths = sorted((scratch/"run-02").glob("*.json")) + [scratch/"semantic-01/semantics.json",
             scratch/"run-01/manifest.json", ROOT/"target/enqueue-receipt-projection-01.log"]
    with ARCHIVE.open("xb") as out, gzip.GzipFile(fileobj=out, filename="", mode="wb", mtime=0) as zipped:
        with tarfile.open(fileobj=zipped, mode="w:") as archive:
            for path in paths:
                data = path.read_bytes()
                assert not any(tag in data.lower() for tag in (b"postgresql://", b"private key-----", b"password="))
                name = path.relative_to(scratch).as_posix() if path.is_relative_to(scratch) else "run-01/failure.log"
                info = tarfile.TarInfo(name)
                info.size = len(data)
                archive.addfile(info, io.BytesIO(data))


def replay():
    schema, _, specs, digest = fixture()
    fingerprints = SUPPORT["sha"]("".join(item[2] for item in specs).encode())
    groups = []
    loads = []
    with tarfile.open(ARCHIVE) as tar:
        def read(name):
            return json.load(tar.extractfile(name))
        manifest = read("run-02/manifest.json")
        assert manifest["arms"] == list(ARMS) and manifest["orders"] == [list(o) for o in ORDERS]
        assert manifest["input"] == digest and manifest["source"] == BASE["FROZEN"]
        assert manifest["source_by_arm"] == {arm:SUPPORT["sha"](schema_for(schema,arm).encode()) for arm in ARMS}
        cal = manifest["calibration"]
        assert cal["mach_timebase"] == [125,3] and abs(cal["mach_ns"]/cal["getrusage_self_ns"]-1) < 0.01
        assert b'row[4] == receipt["submitted_xid"]' in tar.extractfile("run-01/failure.log").read()
        sem = read("semantic-01/semantics.json")
        for arm, value in sem.items():
            first, second = value["first"], value["replacement"]
            assert first == value["replay"] and first["id"] == second["id"] != value["rolled_back_id"]
            assert first["inserted"] and not first["conflict"] and second["conflict"] and not second["inserted"]
            assert value["reflects_inserted_args"] == (arm != "fields")
            assert first["effect_key"] == first["id"] + ("/1/missing" if arm=="fields" else "/1/before")
            assert second["effect_key"] == second["id"]+"/1/after"
        for group, order in enumerate(ORDERS, 1):
            measured = {}
            for i in order:
                arm = ARMS[i]
                rec = read(f"run-02/{group}-{arm}.json")
                METRICS(rec, 125 / 3 / 1e6)
                assert rec["full_receipts_matched"] == 64 and rec["fingerprints_sha256"] == fingerprints
                first = rec["receipt_ids"][0]
                assert rec["replayed_receipt"] == dict(id=first,effect_key=first+"/1",state="available",
                                                       conflict=False,inserted=True)
                stage = rec["stages"]["enqueue"]
                measured[arm] = dict(cpu_ms=sum(stage["backend_cpu_ms"].values()), wall_ms=stage["wall_ms"],
                                    heap_hits=stage["blocks"]["toast_hit"], index_hits=stage["blocks"]["tidx_hit"],
                                    wal_bytes=stage["cluster_wal_bytes"])
                loads.append(rec["load"][0])
            groups.append(measured)
    medians = {a:{k:median(g[a][k] for g in groups) for k in groups[0][a]} for a in ARMS}
    paired = {a:{k:dict(median_ms=median(vals),faster_than_both=sum(v < 0 for v in vals))
                 for k in ("cpu_ms","wall_ms")
                 for vals in [[g[a][k] - min(g["stock_a"][k],g["stock_b"][k]) for g in groups]]}
              for a in ("direct","fields")}
    return dict(groups=groups,medians=medians,paired=paired,load_range=[min(loads),max(loads)],
                archive_sha256=SUPPORT["sha"](ARCHIVE.read_bytes()))


if __name__ == "__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seal",action="store_true")
    args=parser.parse_args()
    if args.seal:
        seal()
    data = json.dumps(replay(),indent=2,sort_keys=True) + "\n"
    if args.seal:
        (HERE/"result.json").write_text(data)
    else:
        assert (HERE/"result.json").read_text()==data
    print(json.dumps(json.loads(data)["paired"]))
