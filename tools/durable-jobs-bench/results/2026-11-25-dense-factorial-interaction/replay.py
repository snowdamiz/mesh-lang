"""Reconstruct the dense factorial contrasts from the two released native blocks."""
from hashlib import sha256
import json
from pathlib import Path
import tarfile

PILOT = Path(__file__).resolve().parent.parent / "2026-11-25-dense-fixed8-logical-factorial"
TRIAL = PILOT / "evidence/native-retry"
ORDER = ["stock-a", "key", "combined", "fixed8", "stock-b"]


def measure(folder, phase, row):
    archive = folder / f"{phase}-mesh/raw.tar.gz"
    assert sha256(archive.read_bytes()).hexdigest() == row["raw_sha256"]
    with tarfile.open(archive) as tar:
        ledger = json.load(tar.extractfile("adapter/ledger.json"))
        assert json.load(tar.extractfile("status.json"))["exit_code"] == 0
    assert ledger["runners"] == 2 and ledger["slots"] == 64
    jobs = ledger["jobs"]
    txn, = ledger["transactions"]
    assert len(jobs) == row["completed"] == 1024
    assert len({j[key] for j in jobs for key in ("attempt_xid", "effect_xid")}) == 2048
    last = max(j["settled_us"] for j in jobs)
    second = sorted(j["effect_committed_us"] for j in jobs)
    clocks = {
        "submit_ms": (last - txn["submitted_us"]) / 1000,
        "drain_ms": (last - txn["committed_us"]) / 1000,
        "enqueue_ms": (txn["committed_us"] - txn["submitted_us"]) / 1000,
        "middle_writes_ms": (second[921] - second[102]) / 1000,
        "last_effect_ms": (second[-1] - txn["committed_us"]) / 1000,
        "terminal_tail_ms": (last - second[-1]) / 1000,
    }
    assert clocks["submit_ms"] == row["submit_us"] / 1000
    assert clocks["drain_ms"] == row["drain_us"] / 1000
    assert row["terminal_xids"] == len({j["settled_xid"] for j in jobs})
    assert not row["pg_cpu"]["unmatched"]
    return clocks | {k: row[k] for k in ("terminal_xids", "claim_sql_calls", "runner_cpu_ms",
                                          "pg_cpu_ms", "host_cpu_ms")}


def replay():
    release, = (TRIAL / "controller").glob("controller-timing-*.json")
    receipt = json.loads(release.read_text())
    assert receipt["exit_status"] == 0 and receipt["groups"] == [0, 1, 2]
    assert receipt["id"] == PILOT.name + "-retry"
    groups = []
    packet_sha = sha256((PILOT / "controller-packet-retry.json").read_bytes()).hexdigest()
    assert packet_sha == "5d97e99284e97944c6dc3b838a8b899a9d7ef24b7c01a0e92cd51c18b2ee7ab7"
    for number, order in ((1, ORDER), (2, ORDER[::-1])):
        block, = (TRIAL / "blocks").glob(f"{number:02}-noop.*")
        assert json.loads((block / "order.json").read_text())["order"] == order
        phases = json.loads((block / "reconciled.json").read_text())
        assert set(phases) == {"measured", "warmup"}
        for phase, scores in phases.items():
            assert set(scores) == set(order)
            measures = {arm: measure(block / f"{phase}-{arm}", phase, scores[arm]) for arm in order}
            if phase == "measured":
                measured = measures
        key, fixed, both = (measured[k] for k in ("key", "fixed8", "combined"))
        extra = {}
        for metric in both:
            # Beyond the sum of single-component savings on this additive scale.
            extra[metric] = [round(key[metric] + fixed[metric] - both[metric] - measured[control][metric], 3)
                             for control in ("stock-a", "stock-b")]
        groups.append(dict(group=number, measured=measured, extra_saving_vs_stock_a_b=extra))
    return groups


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True, allow_nan=False))
