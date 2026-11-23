"""Check the frozen macOS PostgreSQL trial receipts and print their paired prices."""
import argparse
import hashlib
import json
from pathlib import Path
from statistics import median

HERE = Path(__file__).resolve().parent

def collect(directory):
    files = sorted(directory.glob("*.json"))
    if len(files) != 33:
        raise ValueError("expected the run manifest and 32 trial receipts")
    return {path.stem: json.loads(path.read_text()) for path in files}

def reduce(raw):
    m = raw["manifest"]
    calibration = m["calibration"]["mach_timebase"]
    assert hashlib.sha256(m["batch_sha256"].encode()).hexdigest() == "42bb9e2be94c17b36110081430faf2647a04f3a3e0f2decf00e7c216fed462fd"
    assert len(raw) == 33 and sorted(m["orders"][0]) == [0,1,2,3]
    assert len(m["orders"]) == 4 and all(sorted(pos) == [0,1,2,3] for pos in zip(*m["orders"],strict=True))
    prices = {}
    for kind in ["canonical","compressible"]:
        groups = []
        for group,order in enumerate(m["orders"],1):
            arms = {}
            for i in order:
                policy = m["policies"][i]
                r = raw[f"{kind}-{group}-{policy}"]
                bulk = policy != "external"
                assert r["layout"] == bulk and r["jobs"] == r["receipt_rows"] == 512
                assert r["calls"] == dict(bulk_grants=504 if bulk else 0,scalar_grants=8 if bulk else 512,
                                          bulk_calls=8,layout_fallbacks=0 if bulk else 8)
                assert r["settings"]["server_version_num"] == "170011"
                assert r["settings"]["fsync"] == r["settings"]["synchronous_commit"] == "on"
                assert r["settings"]["default_toast_compression"] == ("lz4" if policy == "lz4" else "pglz")
                assert len(r["storage"]) == 1 and sum(n for n,_ in r["storage"].values()) == 512
                assert int(r["top_xid"]) > 0 and r["commit"] and r["wall_ms"] > 0
                cpu = r["backend_ticks"]*calibration[0]/calibration[1]/1e6
                assert cpu > 0 and abs(cpu-r["backend_cpu_ms"]) < 0.000001
                arms[policy] = dict(cpu_ms=cpu,wall_ms=r["wall_ms"],stored_bytes=sum(n for _,n in r["storage"].values()))
            groups.append(arms)
        prices[kind] = {"median":{a:{metric:median(g[a][metric] for g in groups) for metric in groups[0][a]}
                                   for a in m["policies"]},
                        "lz4_saving_against_faster_control":{metric:[min(g["pglz_a"][metric],g["pglz_b"][metric])-g["lz4"][metric] for g in groups]
                                                           for metric in ("cpu_ms","wall_ms")}}
    return prices

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--collect",type=Path,help="reproduce the saved raw packet from a fresh complete run")
    args = parser.parse_args()
    file = HERE / "receipts.json"
    if args.collect:
        raw = collect(args.collect)
        reduce(raw)
        print(json.dumps(raw,indent=2,sort_keys=True))
    else:
        print(json.dumps(reduce(json.loads(file.read_text())),indent=2,sort_keys=True))
