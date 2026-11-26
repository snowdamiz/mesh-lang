"""Place the empty grant body and its following fallback check on the durable ledger."""
from collections import Counter, defaultdict
from decimal import Decimal
import json

from grant_occupancy_cost import PROFILES, grant_capture, nodes


def elapsed_us(plan):
    micros = Decimal(str(plan["duration_ms"])) * 1000
    assert micros == int(micros) and micros >= 0
    return int(micros)


def arm(profile, lane):
    digest, jobs, txn, statements, plans = grant_capture(profile, lane)
    body, = (s for s in statements if s["role"] == "parity_subject"
             and s["query"].startswith("WITH picked AS MATERIALIZED"))
    probe, = (s for s in statements if s["role"] == "parity_subject" and
              s["query"].startswith("EXISTS (") and "batch_context_blocks" in s["query"])
    assert not body["toplevel"] and not probe["toplevel"]
    selected = [p for p in plans if p["session_role"] == "parity_subject" and
                p["query_id"] in (body["queryid"], probe["queryid"])]
    assert Counter(p["query_id"] for p in selected) == {
        body["queryid"]: body["calls"], probe["queryid"]: probe["calls"]}
    by_pid = defaultdict(list)
    for p in selected:
        assert p["end_us"] % 1000 == 0
        by_pid[p["pid"]].append(p)
    effects = sorted(j["effect_committed_us"] for j in jobs)
    lower, upper = effects[len(jobs) // 4 - 1], effects[3 * len(jobs) // 4 - 1]
    settled = max(j["settled_us"] for j in jobs)
    stages = defaultdict(Counter)
    productive = rows = custom = paired = 0
    for group in by_pid.values():
        ordered = iter(sorted(group, key=lambda p: p["log_line"]))
        seen_generic = False
        cold = 0
        for p in ordered:
            assert p["query_id"] == body["queryid"]
            scan, = (n for n in nodes(p["plan"]) if n.get("Index Name") == "mesh_jobs_ready")
            param = "(j.namespace = $1)" in scan["Index Cond"]
            if param:
                seen_generic = True
            else:
                assert not seen_generic and "(j.namespace = '<literal>'::text)" in scan["Index Cond"]
                assert p["end_us"] + 1000 <= txn["committed_us"]
                cold += 1
            returned = p["plan"]["Actual Rows"]
            if returned:
                assert param
                productive += 1
                rows += returned
                continue
            after = next(ordered)
            assert after["query_id"] == probe["queryid"] and after["plan"]["Actual Rows"] == 1
            assert p["log_line"] < after["log_line"] and p["end_us"] <= after["end_us"]
            begin = min(z["end_us"] - elapsed_us(z) for z in (p, after))
            end = after["end_us"] + 1000
            if end <= txn["committed_us"]:
                stage = "before_enqueue_commit"
            elif begin > settled:
                stage = "after_final_settlement"
            elif end <= lower:
                stage = "early_or_enqueue_boundary"
            elif begin > upper:
                stage = "late_or_settlement_boundary"
            else:
                stage = "touches_middle"
            stages[stage]["calls"] += 1
            stages[stage]["body_us"] += elapsed_us(p)
            stages[stage]["probe_us"] += elapsed_us(after)
            paired += 1
        assert cold == 5
        custom += cold
    assert paired == probe["calls"] and productive + paired == body["calls"]
    assert rows == body["rows"] == len(jobs)
    return dict(raw_sha256=digest, login_backends=len(by_pid), custom_before_enqueue=custom,
                parameterized_body_calls=body["calls"] - custom, productive=productive,
                rows=rows, empty=paired, empty_phases=stages)


if __name__ == "__main__":
    print(json.dumps({p: {l: arm(p, l) for l in ("plan-stock", "plan-key")} for p in PROFILES},
                     sort_keys=True, indent=2))
