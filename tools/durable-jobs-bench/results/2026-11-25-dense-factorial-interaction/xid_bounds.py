"""Bound application XID assignment using only ordered IDs and existing clock witnesses."""
import hashlib
import json
from pathlib import Path
import tarfile

ROOT = Path(__file__).resolve().parent.parent
WARM = ROOT / "2026-11-25-dense-fixed8-river-readiness/evidence/native"
DIRECT = ROOT / "2026-11-24-trusted-handler-app-pool/timing-clean-identity-retry/evidence"


def brackets(transactions):
    # Each tuple is (XID, phase, earliest possible assignment, commit stamp).
    ordered = sorted(transactions)
    assert len({x[0] for x in ordered}) == len(ordered)
    assert ordered[0][0] >= 3 and ordered[-1][0] < 2**31
    earliest = []
    for _, _, lower, _ in ordered:
        earliest.append(max(lower, earliest[-1]) if earliest else lower)
    latest = [row[3] for row in ordered]
    for i in reversed(range(len(latest) - 1)):
        latest[i] = min(latest[i], latest[i + 1])
    for row, lower, upper in zip(ordered, earliest, latest):
        assert row[2] <= lower <= upper <= row[3]
        yield (*row, lower, upper)


def measure(path, digest):
    assert hashlib.sha256(path.read_bytes()).hexdigest() == digest
    with tarfile.open(path) as raw:
        assert json.load(raw.extractfile("status.json"))["exit_code"] == 0
        ledger = json.load(raw.extractfile("adapter/ledger.json"))
    jobs = ledger["jobs"]
    assert len(jobs) == len({row["job_key"] for row in jobs}) == 1024
    origin, = ledger["transactions"]
    effects = sorted(j["effect_committed_us"] for j in jobs)
    begin, end = effects[255], effects[767]
    def overlap(left, right):
        return max(0, min(right, end) - max(left, begin))
    transactions = []
    for j in jobs:
        entered, first, second = (j[k] for k in ("entered_us", "attempt_committed_us", "effect_committed_us"))
        a, b = int(j["attempt_xid"]), int(j["effect_xid"])
        assert int(origin["xid"]) < a < b and j["attempt"] == j["attempts"] == j["effects"] == 1
        assert origin["committed_us"] <= entered <= first <= second <= j["settled_us"]
        transactions.extend(((a, "first", entered, first), (b, "second", first, second)))
    phases = {name: dict(total=0, before_min=0, before_max=0, after_min=0, after_max=0)
              for name in ("first", "second")}
    for _, name, start, commit, lower, upper in brackets(transactions):
        count = phases[name]
        count["total"] += overlap(start, commit)
        count["before_min"] += overlap(start, lower)
        count["before_max"] += overlap(start, upper)
        count["after_min"] += overlap(upper, commit)
        count["after_max"] += overlap(lower, commit)
    for counts in phases.values():
        assert counts["before_min"] + counts["after_max"] == counts["total"]
        assert counts["before_max"] + counts["after_min"] == counts["total"]
    claimed_wait = sum(overlap(job["claimed_us"], job["entered_us"]) for job in jobs)
    settled = max(job["settled_us"] for job in jobs)
    return dict(raw_sha256=digest, middle_512_us=end - begin, phases_job_us=phases,
                enqueue_us=origin["committed_us"] - origin["submitted_us"],
                drain_us=settled - origin["committed_us"], submit_us=settled - origin["submitted_us"],
                claim_stamp_to_entry_job_us=claimed_wait)


def replay():
    warm = json.loads((WARM / "replay-rest.json").read_bytes())
    assert json.loads((WARM / "controller/rest-release.json").read_bytes())["exit_status"] == 0
    direct = json.loads((DIRECT / "summary.json").read_bytes())
    assert not direct["missing"] and not direct["failures"]
    for part in ("1-2", "3-6"):
        assert json.loads((DIRECT / f"controller-release-groups-{part}.json").read_bytes())["exit_status"] == 0
    result = {"warm_series": {}, "prior_direct_pool": {}}
    for group in range(1, 7):
        lanes = {}
        for lane in ("combined", "river-warm"):
            row = warm["groups"][str(group)]["measured"][lane]
            file, = WARM.glob(f"blocks/{group:02}-*/measured-{lane}/*/raw.tar.gz")
            lanes[lane] = measure(file, row["raw_sha256"])
            assert lanes[lane]["middle_512_us"] == row["progress"]["second_write"]["middle_512_us"]
            assert all(lanes[lane][k] == row[k] for k in ("enqueue_us", "drain_us", "submit_us"))
        result["warm_series"][group] = lanes
        block, = (b for b in direct["blocks"] if b["profile"] == "noop" and b["group"] == group)
        lanes = {}
        for lane in ("service-a", "service-b", "direct"):
            file, = DIRECT.glob(f"timing/{group:02}-noop/measured-{lane}/*/raw.tar.gz")
            lanes[lane] = measure(file, block["measured"][lane]["raw_sha256"])
        result["prior_direct_pool"][group] = lanes
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
