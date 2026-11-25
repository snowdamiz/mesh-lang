"""Apply archived host-counter bounds to each paired peer's available two-CPU time."""
from hashlib import sha256
import json
from pathlib import Path
from statistics import median
import sys
import tarfile

from native_report import ROOT, LANES, NAMES, check

FROZEN = Path(__file__).resolve().parents[1] / "2026-11-23-ordinary-native-cpu/replay"
sys.path.insert(0, str(FROZEN))
from offline import host_interval


def receipts():
    meta = json.loads((ROOT / "series.json").read_bytes())["config"]
    summary = json.loads((ROOT / "final-replay.json").read_bytes())
    check(meta["cpus"] == [0, 1] and meta["clock_ticks"] == 100, "host accounting configuration differs")
    check(len(summary["blocks"]) == 6 and not summary["missing"], "incomplete matched series")
    groups = []
    for block in summary["blocks"].values():
        rows = {}
        for label in ["hot", *LANES[3:]]:
            name = "measured-" + label
            score = block["scores"][name]
            path, = (ROOT / "blocks" / block["attempt"] / name).glob("*/raw.tar.gz")
            check(sha256(path.read_bytes()).hexdigest() == summary["raw"][str(path.relative_to(ROOT))]
                  == score["raw_sha256"], "changed arm")
            with tarfile.open(path) as raw:
                ledger = json.load(raw.extractfile("adapter/ledger.json"))
                host = json.load(raw.extractfile("host.json"))
                points = [json.loads(line) for line in raw.extractfile("host-samples.jsonl")]
            check(host["affinity"] == [0, 1] and host["boot_id"] == meta["host"][0],
                  "CPU inventory changed")
            jobs = ledger["jobs"]
            check(len(jobs) == score["completed"] == 1024 and score["failed"] == 0, "incomplete jobs")
            check(len({int(j[key]) for j in jobs for key in ("attempt_xid", "effect_xid")}) == 2048,
                  "committed application XIDs differ")
            txn, = ledger["transactions"]
            last = max(j["settled_us"] for j in jobs)
            check(last == score["last_settlement_us"] and txn["submitted_us"] == score["first_submit_us"]
                  and txn["committed_us"] == score["first_commit_us"], "window differs from score")
            writes = sorted(j["effect_committed_us"] for j in jobs)
            phases = {"submit": (txn["submitted_us"], last), "drain": (txn["committed_us"], last),
                      "middle": (writes[102], writes[921])}
            rows[label] = {}
            for phase, edges in phases.items():
                # One denominator unit represents the entire window, so limits are CPU-ms.
                bound = host_interval(points, edges, meta["clock_ticks"], ["cpu0", "cpu1"], 1)
                rows[label][phase] = (bound["host_ms_per_job"]["busy"][0], bound["capacity_ms_per_job"])
        groups.append(rows)
    return groups


def report():
    groups = receipts()
    lines = ["# Host CPU budget at peer completion times", "",
             "Each row compares the hot grouped-settlement Mesh and peer arms from six dense Linode pairs. "
             "The lower bound counts only busy CPU time proven inside the Mesh window. The peer budget "
             "is twice its elapsed time on the same two CPUs, granting it 100% utilization. A positive "
             "difference is the minimum observed CPU service that would have to disappear from that "
             "window for Mesh to match the peer. Scheduling changes can move work across window boundaries.",
             "", "| Peer | Window | Pairs with positive excess | Lower-bound excess CPU-ms, min / median / max |",
             "| --- | --- | ---: | ---: |"]
    for label, name in zip(LANES[3:], NAMES[3:], strict=True):
        for phase in ("submit", "drain", "middle"):
            values = [max(0.0, row["hot"][phase][0] - row[label][phase][1]) for row in groups]
            lines.append(f"| {name} | {phase} | {sum(v > 0 for v in values)}/6 | "
                         f"{min(values):.1f} / {median(values):.1f} / {max(values):.1f} |")
    lines += ["", "Submit and drain end at the last durable settlement. Middle runs from the 103rd through "
              "the 922nd durable second write. Each interval uses its own clock on the same pinned boot. "
              "The archived host estimator excludes boundary samples, allows 2 ms of clock uncertainty "
              "and one accounting tick per CPU and busy counter. Busy includes user, nice, system, IRQ "
              "and softirq service for everything on the pinned CPUs. Some of that service may be unrelated "
              "to Jobs. Zero excess leaves the CPU question open.",
              "", "Run `python3 -B host_capacity.py` to verify the 24 archive hashes and regenerate the table. "
              "[Frozen host estimator](../2026-11-23-ordinary-native-cpu/replay/offline.py) · "
              "[Matched four-engine scores](NATIVE-RESULTS.md)", ""]
    return "\n".join(lines)


if __name__ == "__main__":
    print(report(), end="")
