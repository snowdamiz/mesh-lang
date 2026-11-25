"""Score writer fusion against duplicate off controls and the released winner."""
import json
import tarfile


def middle(values):
    values = sorted(values)
    assert len(values) == 1024
    return values[921] - values[102]


def grant_evidence(jobs, traces, lane, statements):
    by_id = {job["id"]: job for job in jobs}
    by_runner = {job["worker_pid"] for job in jobs}
    observed_runners = set()
    corrected = {}
    fused_turns, total = 0, 0
    top = [row for row in statements if "FROM mesh_jobs.complete_and_refill_v1(" in row["query"]]
    assert len(by_id) == len(jobs)
    if lane == "bridge":
        assert not top and all("standalone_nonempty" not in trace for trace in traces)
        return jobs, dict(fused_seats=0, fused_turns=0)
    assert len(traces) == len(by_runner) == 2
    for trace in traces:
        assert trace["mode"] == ("fused_fixed8ms" if lane in ("fused", "positive") else "fixed8ms")
        assert trace["fused_rejected"] == trace["fused_uncertain"] == 0
        count = 0
        parent_pids = set()
        for parent_id, grant_ids in trace["fused_links"]:
            fused_turns += 1
            parent = by_id[parent_id]
            parent_pids.add(parent["worker_pid"])
            for uid in grant_ids:
                grant = by_id[uid]
                commit = parent["settled_us"]
                assert uid not in corrected and grant["worker_pid"] == parent["worker_pid"]
                assert grant["claimed_us"] <= commit <= grant["entered_us"]
                assert grant["settled_xid"] != parent["settled_xid"]
                corrected[uid] = dict(grant, claimed_us=commit)
                count += 1
        assert len(parent_pids) <= 1 and not observed_runners.intersection(parent_pids)
        observed_runners.update(parent_pids)
        assert count == trace["fused_replacements"]
        total += count
    if lane in ("fused", "positive"):
        assert len(top) == 1
        assert top[0]["calls"] == sum(t["fused_submitted"] for t in traces)
        assert top[0]["calls"] == sum(t["fused_commits"] for t in traces)
        assert top[0]["rows"] >= top[0]["calls"] + total
        assert observed_runners == by_runner and all(t["fused_replacements"] > 0 for t in traces)
    else:
        assert not top and not total
    ordinary = sum(t["standalone_nonempty"] for t in traces)
    return [corrected.get(row["id"], row) for row in jobs], dict(
        fused_seats=total, fused_turns=fused_turns, ordinary_grant_turns=ordinary,
        total_grant_turns=ordinary + fused_turns,
        fused_with_pending=sum(t["fused_with_pending"] for t in traces))


def score(folder, phase, lane, cfg, base):
    run, transfer = base["run"], base["transfer"]
    assigned = base["assigned_inventory"]
    if lane in base["PEERS"]:
        return base["score_arm"](folder, phase, lane, cfg, base["grouped"], base["logical"], {})
    assert run.read_json(folder / "dispatch.json")["exit_code"] == 0
    selected = cfg["lanes"][lane]
    arm = dict(cfg, lanes={"hot": selected})
    with assigned(transfer, selected["inventory"]):
        observed = transfer.score_arm(folder, phase, "hot", arm, None)
    with tarfile.open(folder / "raw.tar.gz") as raw:
        def get(name):
            return run.file_json(raw, f"adapter/{name}.json")
        meta = get("logical-overlay")
        base["overlay"](meta, lane, base["logical"].CANDIDATE.read_text())
        ledger, row, queue = get("ledger"), get("row"), get("queue-policy")
        base["verify_slots"](ledger, row)
        jobs = ledger["jobs"]
        statements = queue["queue_statements"]
        traces = [json.loads(line.removeprefix("jobs_grouped_writer "))
                  for i in range(2) for line in transfer.text(raw, f"adapter/runner-{i}.log").splitlines()
                  if line.startswith("jobs_grouped_writer ")]
        assert len(traces) == 2
        assert sum(t["confirmed_free_claims"] for t in traces) == queue["admitted_queries"]
        assert sum(sum(t["cohort_submissions_by_width"][1:]) for t in traces) >= queue["completion_cohort_queries"]
        jobs_for_capacity, grants = grant_evidence(jobs, traces, lane, statements)
        peaks = base["grouped"].occupancy(jobs_for_capacity)
        assert observed["physical_tls_by_role"]["compiled_caller"] == 32
        assert observed["checkout_probes"] < 32 and observed["peak_connections"] <= 41
        if lane == "positive":
            proof = get("grant-contract")
            assert proof["checks"] and all(proof["checks"].values())
            assert proof["contraction"] == base["CONTRACTION"]
            assert grants["fused_seats"] >= 128
            return dict(raw_sha256=observed["raw_sha256"], completed=observed["completed"],
                        peaks=peaks, grant_contract=proof, **grants)
        bracket = base["logical"].cpu(get("pg-cpu-bracket"), observed["first_submit_us"], observed["last_settlement_us"])
        assert not bracket["unmatched"]
    return dict(base["measure"](observed, bracket), **grants, peaks=peaks,
                claim_selects=queue["admitted_queries"],
                queue_sql_calls=sum(row["calls"] for row in statements),
                queue_sql_ms=sum(row["execution_ms"] for row in statements),
                first_write_middle_us=middle([job["attempt_committed_us"] for job in jobs]),
                second_write_middle_us=middle([job["effect_committed_us"] for job in jobs]),
                terminal_middle_us=middle([job["settled_us"] for job in jobs]))


def gate(rows):
    a, b, bridge, fused = (rows[name] for name in ("off-a", "off-b", "bridge", "fused"))
    metrics = {}
    for key, fraction in (("submit_us", .05), ("drain_us", .05), ("first_write_middle_us", .03),
                          ("second_write_middle_us", .03), ("terminal_middle_us", .03)):
        faster = min(a[key], b[key], bridge[key])
        spread = abs(a[key] - b[key])
        saving = faster - fused[key]
        threshold = max(fraction * faster, spread, 10_000)
        bridge_delta = max(abs(bridge[key] - value[key]) for value in (a, b))
        metrics[key] = dict(saving=saving, threshold=threshold, bridge_delta=bridge_delta,
                            passed=saving > threshold and bridge_delta <= max(.05 * bridge[key], spread))
    resources = {}
    for key in ("runner_cpu_ms", "host_cpu_ms", "pg_cpu_ms", "queue_sql_calls"):
        ceiling = min(max(a[key], b[key]), bridge[key] * 1.03)
        resources[key] = dict(value=fused[key], ceiling=ceiling, passed=fused[key] <= ceiling)
    checks = dict(
        actual_replacements=fused["fused_seats"] >= 256,
        fewer_standalone_grant_turns=fused["ordinary_grant_turns"] < .85 * min(a["ordinary_grant_turns"], b["ordinary_grant_turns"]),
        fewer_total_grant_turns=fused["total_grant_turns"] < min(a["total_grant_turns"], b["total_grant_turns"]),
        terminal_xids=fused["terminal_xids"] <= max(a["terminal_xids"], b["terminal_xids"], bridge["terminal_xids"]),
        last_effect=fused["last_effect_from_commit_us"] < min(a["last_effect_from_commit_us"],
                                                             b["last_effect_from_commit_us"], bridge["last_effect_from_commit_us"]),
        fewer_standalone_selects=fused["claim_selects"] < min(a["claim_selects"], b["claim_selects"], bridge["claim_selects"]),
    )
    return dict(metrics=metrics, resources=resources, checks=checks,
                passed=all(checks.values()) and all(item["passed"] for item in (*metrics.values(), *resources.values())))
