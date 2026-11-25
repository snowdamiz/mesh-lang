"""Replay sampled grouped-settlement plans and the separate fixed8 workload counters."""
import hashlib
import json
from collections import Counter
from pathlib import Path
import tarfile


RESULTS = Path(__file__).resolve().parents[1]
PLANS = RESULTS / "2026-11-25-dense-logical-grant-key-native/evidence/native"
FIXED8 = RESULTS / "2026-11-25-dense-fixed8-river-readiness/evidence/native"


def archive(root, group, lane, digest):
    path, = root.glob(f"blocks/{group:02}-*/measured-{lane}/*/raw.tar.gz")
    assert hashlib.sha256(path.read_bytes()).hexdigest() == digest
    return tarfile.open(path)


def read(raw, member):
    return json.load(raw.extractfile(member))


def nodes(plan):
    yield plan
    for child in plan.get("Plans", []):
        yield from nodes(child)


def drain(raw):
    assert read(raw, "status.json")["exit_code"] == 0
    ledger = read(raw, "adapter/ledger.json")
    jobs = ledger["jobs"]
    assert len(jobs) == len({job["id"] for job in jobs}) == 1024
    assert len({job[k] for job in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
    origin, = ledger["transactions"]
    return origin["committed_us"], max(job["settled_us"] for job in jobs), jobs


def sampled(raw):
    begin, end, _ = drain(raw)
    totals = Counter()
    widths = Counter()
    sessions = set()
    for sample in read(raw, "adapter/grant-plans.json"):
        plan = sample["plan"]
        if not any(n.get("Subplan Name") == "CTE completed" for n in nodes(plan)):
            continue
        if sample["end_us"] > end or sample["end_us"] - 1000 * sample["duration_ms"] < begin:
            continue
        assert sample["session_role"] == "parity_subject" and sample["database"] == "mesh"
        locked, = (n for n in nodes(plan) if n.get("Subplan Name") == "CTE held")
        assert locked["Node Type"] == "LockRows"
        broad, = (n for n in nodes(locked) if n.get("Relation Name") == "jobs")
        assert broad["Index Name"] == "mesh_jobs_expiring" and broad["Index Cond"] == "(j.namespace = $1)"
        target, = (n for n in nodes(plan) if n.get("Index Name") == "jobs_pkey")
        assert ".id =" in target["Index Cond"] and ".namespace =" in target["Index Cond"]
        widths[plan["Actual Rows"]] += 1
        sessions.add(sample["pid"])
        totals["held_scan_rows"] += broad["Actual Rows"] * broad["Actual Loops"]
        totals["held_scan_shared_hits"] += broad["Shared Hit Blocks"]
        totals["update_lookup_rows"] += target["Actual Rows"] * target["Actual Loops"]
        totals["update_lookup_shared_hits"] += target["Shared Hit Blocks"]
        totals["settled_rows"] += plan["Actual Rows"]
        totals["sample_elapsed_us"] += round(sample["duration_ms"] * 1000)
    assert len(sessions) == 2 and totals["update_lookup_rows"] == totals["settled_rows"]
    return dict(totals=totals, width_counts=dict(sorted(widths.items())), session_pids=sorted(sessions))


def replay():
    source = json.loads((PLANS / "final-replay.json").read_bytes())
    measured = source["groups"]["0"]["measured"]
    result = {"sampled_plans": {}, "fixed8_groups": {}}
    for lane in ("plan-stock", "plan-key"):
        digest = measured[lane]["raw_sha256"]
        with archive(PLANS, 0, lane, digest) as raw:
            result["sampled_plans"][lane] = dict(raw_sha256=digest, **sampled(raw))
    matched = json.loads((FIXED8 / "replay-rest.json").read_bytes())["groups"]
    for i in range(1, 7):
        arm = matched[str(i)]["measured"]["combined"]
        river = matched[str(i)]["measured"]["river-warm"]
        with archive(FIXED8, i, "combined", arm["raw_sha256"]) as raw:
            begin, end, jobs = drain(raw)
            policy = read(raw, "adapter/queue-policy.json")
        with archive(FIXED8, i, "river-warm", river["raw_sha256"]) as raw:
            river_begin, river_end, _ = drain(raw)
        assert river_end - river_begin == river["drain_us"]
        complete, = (s for s in policy["queue_statements"] if "FROM mesh_jobs.complete_batch_v1(" in s["query"])
        cohorts = Counter(Counter(j["settled_xid"] for j in jobs).values())
        scalar = sum(s["calls"] for s in policy["queue_statements"] if "coalesce(mesh_jobs.settle_state(" in s["query"])
        assert complete["rows"] + scalar == 1024 and end - begin == arm["drain_us"]
        assert complete["calls"] == policy["completion_cohort_queries"] == sum(cohorts.values()) - scalar
        assert max(cohorts) <= 8
        result["fixed8_groups"][i] = dict(raw_sha256=arm["raw_sha256"], river_raw_sha256=river["raw_sha256"],
                                         cohort_counts=dict(sorted(cohorts.items())),
                                         grouped_calls=complete["calls"], scalar_calls=scalar, inclusive_sql_ms=complete["execution_ms"],
                                         warm_river_drain_lead_us=arm["drain_us"] - river["drain_us"])
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
