"""Reconcile canonical checked, individual-settlement arms and their plans."""

from collections import Counter
import json
import re

from audit import ELF, lifecycle, member
from plans import records, reduce
from roster8 import RUNNERS, bind

DIAGNOSTIC = ("plan-stock", "plan-key")
TRIPLE = ("stock-a", "key", "stock-b")


def workload(archive):
    ledger, writer, queue, raw, identity, sessions, settings = (
        member(archive, name) for name in ("ledger", "producer-writer", "queue-policy", "raw",
                                         "process-identity", "postgres-tls-sessions", "storage-topology"))
    jobs, txn = lifecycle(ledger, writer, queue)
    assert (ledger["runners"], ledger["slots"]) == (8, 64)
    assert 0 < ledger["peak_executing"] <= 64 and ledger["peak_connections"] == 65
    db = ledger["database"]
    assert db["version"] == "17.11" and db["max_connections"] == db["database_connection_limit"] == 96
    assert db["reserved_connections"] == db["superuser_reserved_connections"] == 0
    assert all(db[key] is True for key in ("fsync", "full_page_writes", "synchronous_commit",
                                          "track_commit_timestamp", "tls"))
    assert (settings["topology"], settings["storage"], settings["work_mem"]) == ("stock", "default", "4MB")
    assert member(archive, "build")["files"]["mesh"] == ELF
    assert raw["idle_seconds"] >= 3 and raw["raw_idle_sql_calls"] > 0
    assert raw["poll_ms"] == 250 and raw["statement_counter_deallocations"] == 0
    assert len(raw["event_drops"]) == RUNNERS
    assert not any(value for row in raw["event_drops"] for value in row.values())
    assert len(identity) == 9 and len({row["pid"] for row in identity}) == 9
    assert all(row["executable_sha256"] == ELF and row["cpus"] == [0, 1]
               and row["launch_start_ticks"] == row["start_ticks_before"] == row["start_ticks_after"]
               for row in identity)
    broker, *runners = identity
    pids = [row["pid"] for row in runners]
    assert broker["argv"][-1] == "session" and all(row["argv"][-1] == "run" for row in runners)
    assert {row["worker_pid"] for row in jobs} == set(pids)
    assert Counter(row["role"] for row in sessions)["compiled_caller"] == 32
    assert Counter(row["role"] for row in sessions)["parity_producer"] == 1
    assert all(row["tls"] is True for row in sessions)
    app = member(archive, "trusted-application-statements")
    assert sum(s["calls"] for s in app if s["query"] == "SELECT $1") == 2048
    claim, = [s for s in queue["queue_statements"] if "FROM mesh_jobs.claim_batch_set_v1(" in s["query"]]
    assert not [s for s in queue["queue_statements"] if "FROM mesh_jobs.claim" in s["query"] and s != claim]
    assert claim["calls"] == queue["admitted_queries"] >= raw["raw_claim_calls"] > 0
    assert claim["rows"] >= 1024
    reports = []
    for i in range(RUNNERS):
        report = json.loads(archive.extractfile(f"adapter/runner-{i}.log").read().splitlines()[-1])
        assert report["drained"] and report["last_error"] is None and report["pending_claims"] == 0
        assert report["queues"]["default"]["limit"] == 8
        assert report["claimed"] == report["finished"] == sum(job["worker_pid"] == pids[i] for job in jobs)
        reports.append(dict(pid=pids[i], jobs=report["claimed"], hints=report["batch_contention_hints"]))
    log = archive.extractfile("adapter/postgres.log").read().decode()
    hints = sum(report["hints"] for report in reports)
    queue_timeouts = re.findall(
        r"ERROR:\s+canceling statement due to lock timeout\r?\n"
        r"[^\n]+CONTEXT:[^\n]+FROM mesh_jobs\.queues\r?\n"
        r'[^\n]+FOR NO KEY UPDATE"\r?\n'
        r"[^\n]+mesh_jobs\.claim_batch_set_v1[^\n]+line 20 at SQL statement", log)
    assert hints == log.count("canceling statement due to lock timeout") == len(queue_timeouts)
    end = max(job["settled_us"] for job in jobs)
    last_effect = max(job["effect_committed_us"] for job in jobs)
    return dict(first_submit_us=txn["submitted_us"], first_commit_us=txn["committed_us"],
                last_effect_us=last_effect, last_settlement_us=end, runners=reports,
                configured_seats=64, peak_executing=ledger["peak_executing"], cap=96,
                peak_connections=ledger["peak_connections"], checked_checkout_probes=2048,
                submit_us=end-txn["submitted_us"], drain_us=end-txn["committed_us"],
                effect_from_submit_us=last_effect-txn["submitted_us"],
                effect_from_commit_us=last_effect-txn["committed_us"],
                claim_calls=raw["raw_claim_calls"], lifetime_claim_calls=claim["calls"],
                lifetime_claim_elapsed_ms=claim["execution_ms"], queue_lock_timeouts=hints)


def visits(scan):
    return scan["emitted_approx"] + sum(scan["removed_approx"].get(k, 0) for k in (
        "Rows Removed by Filter", "Rows Removed by Index Recheck"))


def subtree(nodes, subplan):
    root, = [node for node in nodes if node.get("Subplan Name") == subplan]
    path = root["subtree_path"]
    return root, [node for node in nodes if node["subtree_path"][:len(path)] == path and node is not root]


def grant_plans(observation, candidate):
    runners = {pid: runner for runner, pids in observation["participants"].items() for pid in pids}
    counts = {runner: Counter() for runner in observation["participants"]}
    indexes = Counter()
    empty = 0
    for sample in observation["samples"]:
        if sample["category"] != "grant" or sample["window"] not in {"idle", "drain"}:
            continue
        nodes = sample["nodes"]
        root, = [node for node in nodes if not node["subtree_path"]]
        target, update = subtree(nodes, "CTE granted")
        picked, pick = subtree(nodes, "CTE picked")
        assert target["Node Type"] == "ModifyTable"
        granted = root["emitted_approx"]
        assert target["emitted_approx"] == granted
        targets = [node for node in update if node.get("Relation Name") == "jobs"]
        picks = [node for node in pick if node.get("Relation Name") == "jobs"]
        assert targets and picks
        target_visits = sum(map(visits, targets))
        bitmap_entries = sum(n["emitted_approx"] for n in update if n["Node Type"] == "Bitmap Index Scan")
        if not granted:
            empty += 1
            if candidate:
                assert target_visits == bitmap_entries == 0
                assert not any(n["Actual Loops"] for n in targets)
            continue
        assert sample["window"] == "drain" and 1 <= granted <= 8
        key, = sample["statement_keys"]
        assert key["toplevel"] is False and key["role"] == "parity_subject"
        if candidate:
            assert target_visits <= 2*granted and bitmap_entries <= 2*granted
        counts[runners[sample["pid"]]].update(dict(
            calls=1, jobs=granted, broad=int(max(target_visits, bitmap_entries) > 2*granted),
            target_visits=target_visits, target_bitmap_entries=bitmap_entries,
            target_buffer_accesses=sum(n.get("Shared Hit Blocks", 0) + n.get("Shared Read Blocks", 0) for n in targets),
            pick_visits=sum(map(visits, picks)),
            pick_buffer_accesses=picked.get("Shared Hit Blocks", 0) + picked.get("Shared Read Blocks", 0),
            target_join_rejections=sum(n["removed_approx"].get("Rows Removed by Join Filter", 0) for n in update)))
        indexes.update(n["Index Name"] for n in update if n.get("Index Name") and n["Actual Loops"])
    total = Counter()
    for row in counts.values():
        total.update(row)
    assert total["jobs"] <= 1024
    return dict(per_runner={runner: dict(row) for runner, row in counts.items()},
                totals=dict(total), empty=empty, target_indexes=dict(indexes))


ACQUIRED = re.compile(r"^LOG:\s+process \d+ acquired .+ after ([\d.]+) ms")


def queue_waits(log, pids, commit, end):
    pending = {}
    waits = []
    for stamp, pid, line, role, db, lines in records(log):
        if pid not in pids or (role, db) != ("parity_subject", "mesh"):
            continue
        if match := ACQUIRED.match(lines[0]):
            pending[pid] = (stamp, line, float(match[1]))
        elif pid in pending:
            since, previous, duration = pending.pop(pid)
            if (line == previous + 1 and stamp == since and 'relation "queues"' in lines[0]
                    and commit + 1000 <= stamp - duration*1000 <= stamp <= end - 1000):
                waits.append(dict(pid=pid, end_us=stamp, elapsed_ms=duration))
    return waits


def diagnostic(archive, clocks, meta, setup, candidate):
    contract = member(archive, "grant-contract")
    assert contract["overlay"]["body_sha256"] == meta["body_sha256"] and all(contract["checks"].values())
    frames, closed = member(archive, "grant-frames"), member(archive, "grant-closed")
    witness = member(archive, "grant-roster")
    pinned, _ = bind(witness, frames)
    assert witness["runners_by_index"] == [row["pid"] for row in clocks["runners"]]
    submit, commit, end = (clocks[key] for key in ("first_submit_us", "first_commit_us", "last_settlement_us"))
    assert frames[0]["end_us"] < frames[1]["begin_us"] <= frames[1]["end_us"] < submit
    assert frames[1]["us_epoch"] - frames[0]["us_epoch"] >= 2_900_000
    assert end < frames[2]["us_epoch"] <= closed["us_epoch"]
    assert frames[1]["relation"]["n_tup_ins"] == frames[1]["relation"]["n_tup_upd"] == 0
    assert closed["relation"]["n_tup_ins"] == 1024 and closed["relation"]["n_tup_upd"] == 2048
    assert not any(row["usename"] in {"parity_subject", "compiled_caller", "parity_producer"}
                   for row in closed["sessions"])
    settings = member(archive, "grant-observer")
    assert (settings["module"], settings["module_sha256"]) == (setup["remote"], setup["sha256"])
    role_settings = dict(settings["role_settings"])["parity_subject"]
    assert {"auto_explain.sample_rate=1", "log_lock_waits=on", "deadlock_timeout=1ms"} <= set(role_settings)
    plans = member(archive, "grant-plans")
    assert all(row["pid"] in pinned and pinned[row["pid"]][0] == "parity_subject" for row in plans)
    observation = reduce(plans, frames, closed, clocks)
    summary = grant_plans(observation, candidate)
    log = archive.extractfile("adapter/postgres.log").read().decode()
    waits = queue_waits(log, pinned, commit, end)
    lock_rows = [row for row in observation["statement_deltas"] if row["category"] == "queue_lock"
                 and row["role"] == "parity_subject" and not row["toplevel"]
                 and " ".join(row["query"].lower().split()).startswith("select * from mesh_jobs.queues ")]
    assert sum(row["observed_delta"]["calls"] for row in lock_rows) >= clocks["claim_calls"]
    summary.update(
        acquired_queue_wait_count=len(waits), acquired_queue_wait_ms=sum(w["elapsed_ms"] for w in waits),
        acquired_queue_wait_max_ms=max((w["elapsed_ms"] for w in waits), default=0),
        queue_lock_stmt_calls=sum(row["observed_delta"]["calls"] for row in lock_rows),
        queue_lock_stmt_elapsed_ms=sum(row["observed_delta"]["total_exec_time"] for row in lock_rows),
        jobs_index_fetches_observed=frames[2]["relation"]["idx_tup_fetch"]-frames[1]["relation"]["idx_tup_fetch"],
        indexes=observation["indexes"], sample_counts=observation["sample_counts"])
    return summary


def plan_gate(stock, key):
    for arm in (stock, key):
        if (arm["empty"] < 1 or len(arm["per_runner"]) != RUNNERS or
                any(row.get("jobs", 0) == 0 for row in arm["per_runner"].values()) or
                arm["totals"].get("jobs", 0) < 768):
            return dict(passed=False, reason="insufficient live grant coverage")
    a, b = (arm["totals"] for arm in (stock, key))
    normalized = [max(row.get("target_visits", 0), row.get("target_bitmap_entries", 0))/row["jobs"]
                  for row in (a, b)]
    broad_share = a.get("broad", 0) / a["calls"]
    return dict(passed=broad_share >= .25 and normalized[1] <= .25*normalized[0] and b.get("broad", 0) == 0,
                stock_broad_share=broad_share, target_work_per_granted_row=normalized)


def pilot_gate(groups):
    if set(groups) != {0, 1, 2} or not groups[0]["plan_gate"]["passed"]:
        return dict(passed=False, reason="live qualification or duplicate controls incomplete")
    rotations = {}
    for group in (1, 2):
        arms = groups[group]["arms"]
        a, key, b = (arms[lane] for lane in TRIPLE)
        metrics = {}
        for metric in ("submit_us", "drain_us", "effect_from_commit_us",
                       "runner_cpu_ms", "pg_cpu_ms", "host_cpu_ms"):
            faster = min(a[metric], b[metric])
            spread = abs(a[metric] - b[metric])
            saving = faster - key[metric]
            metrics[metric] = dict(saving=saving, duplicate_spread=spread, baseline=faster,
                                   passed=saving > max(.05*faster, spread))
        rotations[group] = dict(metrics=metrics, passed=all(m["passed"] for m in metrics.values()))
    return dict(passed=all(r["passed"] for r in rotations.values()), rotations=rotations)
