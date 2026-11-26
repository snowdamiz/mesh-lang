"""Reconcile the capacity count inside the queue turn with the durable effect ledger."""
from collections import Counter
from decimal import Decimal
from hashlib import sha256
from io import BytesIO
import json
from pathlib import Path
import tarfile

from scalar_lookup_profiles import PROFILES, RESULTS, blob, nodes, read


def arm(profile, lane):
    commit, directory, count = PROFILES[profile]
    root = RESULTS / directory
    binding = json.loads(blob(commit, root / "continuation/qualification.json"))
    member = f"measured-{lane}/measured-mesh"
    if profile == "slow":
        digest = binding["raw_sha256"][member]
    else:
        digest = binding["arms"][lane]["raw_sha256"]
    path = root / "evidence/native/blocks" / Path(binding["block"]).name / member / "raw.tar.gz"
    data = blob(commit, path)
    assert binding["plan_gate"]["passed"] and sha256(data).hexdigest() == digest
    with tarfile.open(fileobj=BytesIO(data)) as archive:
        assert read(archive, "status.json")["exit_code"] == 0
        jobs = read(archive, "adapter/ledger.json")["jobs"]
        statements = read(archive, "adapter/grant-closed.json")["statements"]
        plans = read(archive, "adapter/grant-plans.json")
    assert len(jobs) == len({j["job_key"] for j in jobs}) == count
    assert all(j["attempt"] == j["attempts"] == j["effects"] == 1 and j["state"] == "completed" for j in jobs)
    assert len({j[key] for j in jobs for key in ("attempt_xid", "effect_xid", "settled_xid")}) == 3 * count
    effects = sorted(j["effect_committed_us"] for j in jobs)
    first, last = effects[count // 4 - 1], effects[3 * count // 4 - 1]
    stat, = (s for s in statements if s["role"] == "parity_subject" and
             "q.concurrency - (SELECT count(*) FROM mesh_jobs.jobs" in s["query"])
    samples = [p for p in plans if p["query_id"] == stat["queryid"] and p["session_role"] == stat["role"]]
    assert len(samples) == stat["calls"] and not stat["toplevel"]
    observed = Counter()
    indexes = Counter()
    middle = Counter()
    hits = filtered = 0
    for p in samples:
        scan, = (n for n in nodes(p["plan"]) if n.get("Relation Name") == "jobs")
        assert scan["Node Type"] == "Index Scan" and scan["Actual Loops"] == 1
        assert "count(*)" in next(n for n in nodes(p["plan"]) if n["Node Type"] == "Aggregate")["Output"]
        observed[scan["Actual Rows"]] += 1
        indexes[scan["Index Name"]] += 1
        filtered += scan["Rows Removed by Filter"]
        hits += scan["Shared Hit Blocks"]
        stamp = p["end_us"]
        assert stamp % 1000 == 0
        duration = int(Decimal(str(p["duration_ms"])) * 1000)
        # Treat each millisecond log timestamp as an end-time bin and include
        # the full reported duration if its possible interval touches the window.
        if stamp - duration <= last and stamp + 1000 > first:
            middle["calls"] += 1
            middle["logged_us"] += duration
    return dict(raw_sha256=digest, calls=stat["calls"], executor_us=round(stat["total_exec_time"] * 1000),
                scan_index_calls=indexes, scan_hits=hits, scan_filtered=filtered,
                counted_rows=sum(k * v for k, v in observed.items()), largest_count=max(observed),
                empty_counts=observed[0], middle_log_bins=middle, middle_effect_span_us=last - first)


if __name__ == "__main__":
    print(json.dumps({p: {lane: arm(p, lane) for lane in ("plan-stock", "plan-key")} for p in PROFILES},
                     sort_keys=True, indent=2))
