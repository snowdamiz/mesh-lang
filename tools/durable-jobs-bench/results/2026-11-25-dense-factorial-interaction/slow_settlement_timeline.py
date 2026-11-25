"""Place each scalar settlement scan on the native SLOW plan-capture timeline."""
from collections import Counter, defaultdict
from hashlib import sha256
from io import BytesIO
import json
from pathlib import Path
import tarfile

from scalar_lookup_profiles import PROFILES, RESULTS, STATEMENTS, blob, nodes, read


def report_group(rows):
    return {
        "transactions": len(rows),
        "lock_filtered": sum(r["lock_filtered"] for r in rows),
        "update_filtered": sum(r["update_filtered"] for r in rows),
        "lock_plus_update_diagnostic_ms": round(sum(r["nested_ms"] for r in rows), 3),
    }



def post_select_windows(plans, jobs, rows):
    groups = defaultdict(list)
    for job in jobs:
        groups[job["claimed_us"]].append(job)
    selects = sorted(
        (p for p in plans if str(p["query_id"]) == "8762970956571263497"),
        key=lambda p: p["log_line"],
    )
    ctes = sorted(
        (p for p in plans if "WITH picked AS MATERIALIZED" in p["query"]),
        key=lambda p: p["log_line"],
    )
    assert len({p["pid"] for p in selects + ctes}) == 1
    assert len(selects) == len(ctes)
    productive = []
    grant_targets = {"productive": Counter(), "empty": Counter()}
    for i, (cte, result) in enumerate(zip(ctes, selects)):
        assert cte["log_line"] < result["log_line"]
        assert i == 0 or selects[i-1]["log_line"] < cte["log_line"]
        granted, = [node for node in nodes(cte["plan"]) if node.get("Subplan Name") == "CTE granted"]
        target, = [node for node in nodes(granted) if node.get("Relation Name") == "jobs"
                   and "Scan" in node["Node Type"] and node["Alias"] == granted["Alias"]]
        assert target["Node Type"] == "Index Scan" and target["Index Name"] == "jobs_pkey"
        rows_per_loop = target["Actual Rows"]
        # An equality on the full physical primary key returns at most one row per probe.
        # Actual Rows is rounded per loop. Check full-key productive calls against output width.
        loops = target["Actual Loops"]
        if loops > 1:
            assert "id =" in target["Index Cond"] and loops == cte["plan"]["Actual Rows"] and rows_per_loop == 1
        assert not target.get("Rows Removed by Filter", 0)
        bucket = grant_targets["productive" if cte["plan"]["Actual Rows"] else "empty"]
        bucket.update(calls=1, rows=rows_per_loop * loops, shared_hits=target["Shared Hit Blocks"])
        if cte["plan"]["Actual Rows"] > 0:
            assert cte["plan"]["Actual Rows"] == result["plan"]["Actual Rows"]
            productive.append(result)
    stamps = sorted(groups)
    assert len(productive) == len(stamps)
    covered = set()
    group_counts = []
    weighted_us = 0
    weighted_upper_us = 0
    stamp_to_entry_us = 0
    for stamp, result in zip(stamps, productive):
        group = groups[stamp]
        first_entry = min(j["entered_us"] for j in group)
        assert result["plan"]["Actual Rows"] == len(group)
        assert stamp <= result["end_us"] <= first_entry
        after_log = result["end_us"] + 1000
        # A scalar call began after its predecessor committed on this single backend.
        # Retain only whole transactions strictly between the log and first entry.
        inside = {i for i in range(1, len(rows)) if
                  after_log <= rows[i-1]["commit_us"] and rows[i]["commit_us"] < first_entry}
        covered.update(inside)
        group_counts.append(len(inside))
        weighted_us += len(group) * max(0, first_entry - after_log)
        weighted_upper_us += len(group) * (first_entry - result["end_us"])
        stamp_to_entry_us += len(group) * (first_entry - stamp)
    return {
        "productive_selects": len(productive),
        "grant_update_target": grant_targets,
        "groups_with_whole_older_settlement": sum(n > 0 for n in group_counts),
        "max_whole_settlements_per_group": max(group_counts),
        "distinct_whole_settlements": report_group([rows[i] for i in sorted(covered)]),
        "post_select_to_first_entry_normalized_slot_ms": [
            round(weighted_us / 64000, 3), round(weighted_upper_us / 64000, 3)
        ],
        "ledger_stamp_to_first_entry_normalized_slot_ms": round(stamp_to_entry_us / 64000, 3),
    }


def timeline(raw):
    jobs = sorted(read(raw, "adapter/ledger.json")["jobs"], key=lambda j: j["settled_us"])
    assert len(jobs) == len({j["settled_xid"] for j in jobs}) == len({j["settled_us"] for j in jobs}) == 512
    plans = read(raw, "adapter/grant-plans.json")
    statements = {**STATEMENTS, "outer": "-7725573251209787463"}
    calls = {
        name: sorted((r for r in plans if str(r["query_id"]) == qid), key=lambda r: r["log_line"])
        for name, qid in statements.items()
    }
    assert all(len(rs) == len(jobs) for rs in calls.values())
    assert len({r["pid"] for rs in calls.values() for r in rs}) == 1
    rows = []
    for i, job in enumerate(jobs):
        lock, update, outer = (calls[k][i] for k in ("lock", "update", "outer"))
        assert lock["log_line"] < update["log_line"] < outer["log_line"]
        assert lock["end_us"] <= update["end_us"] <= outer["end_us"] <= job["settled_us"]
        if i:
            assert calls["outer"][i-1]["log_line"] < lock["log_line"]
            # PG log timestamps are floored to milliseconds.
            assert jobs[i-1]["settled_us"] < lock["end_us"] + 1000
        row = {"commit_us": job["settled_us"], "nested_ms": lock["duration_ms"] + update["duration_ms"]}
        for name, record in (("lock", lock), ("update", update)):
            scan, = [n for n in nodes(record["plan"]) if n.get("Relation Name") == "jobs" and "Scan" in n["Node Type"]]
            assert scan["Index Name"] == "mesh_jobs_unique_candidates" and scan["Actual Loops"] == 1
            row[name + "_filtered"] = scan["Rows Removed by Filter"]
        rows.append(row)
    last_stamp = max(j["claimed_us"] for j in jobs)
    assert all(j["claimed_us"] % 1000 == 0 for j in jobs)
    first_final_entry = min(j["entered_us"] for j in jobs if j["claimed_us"] == last_stamp)
    assert first_final_entry >= last_stamp
    before = [r for r in rows if r["commit_us"] < last_stamp]
    during = [r for r in rows if last_stamp <= r["commit_us"] < last_stamp + 1000]
    after = [r for r in rows if r["commit_us"] >= last_stamp + 1000]
    return {
        "post_select_windows": post_select_windows(plans, jobs, rows),
        "by_128_terminal_commits": [report_group(rows[i:i+128]) for i in range(0, 512, 128)],
        "terminal_commits_before_last_claim_stamp": report_group(before),
        "terminal_commits_in_last_claim_stamp_millisecond": report_group(during),
        "terminal_commits_after_last_claim_stamp_millisecond": report_group(after),
        "all": report_group(rows),
        "distinct_claim_milliseconds": len({j["claimed_us"] for j in jobs}),
        "jobs_with_last_claim_stamp": sum(j["claimed_us"] == last_stamp for j in jobs),
        "terminal_commits_before_first_handler_entry_in_last_stamp": sum(r["commit_us"] < first_final_entry for r in rows),
        "diagnostic_final_effect_to_terminal_ms": (rows[-1]["commit_us"] - max(j["effect_committed_us"] for j in jobs))/1000,
    }


def main():
    commit, directory, _ = PROFILES["slow"]
    folder = RESULTS / directory
    binding = json.loads(blob(commit, folder / "continuation/qualification.json"))
    assert binding["plan_gate"]["passed"]
    result = {}
    for lane in ("plan-stock", "plan-key"):
        member = f"measured-{lane}/measured-mesh"
        data = blob(commit, folder / "evidence/native/blocks" / Path(binding["block"]).name / member / "raw.tar.gz")
        digest = sha256(data).hexdigest()
        assert digest == binding["raw_sha256"][member]
        with tarfile.open(fileobj=BytesIO(data)) as raw:
            assert read(raw, "status.json")["exit_code"] == 0
            result[lane] = {"raw_sha256": digest, **timeline(raw)}
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
