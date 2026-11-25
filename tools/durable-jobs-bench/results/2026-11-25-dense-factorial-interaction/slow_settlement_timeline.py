"""Place each scalar settlement scan on the native SLOW plan-capture timeline."""
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
