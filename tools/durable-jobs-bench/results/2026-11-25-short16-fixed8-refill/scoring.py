"""Check native SHORT16 custody and compare durable timestamps."""
from collections import Counter
import json
from statistics import median
import tarfile

from contract import body, digest, STOCK

JOBS = 128
SLOTS = 16
CLOCKS = ("submit_us", "drain_us")
CPU = ("runner_cpu_ms", "pg_cpu_ms", "host_cpu_ms")
TERMINAL = "SELECT coalesce(mesh_jobs.settle_state("
COHORT = "SELECT ordinal::text FROM mesh_jobs.complete_batch_v1("
GRANT = "FROM mesh_jobs.claim_batch_set_v1("


def effects(jobs, score):
    origin = score["first_commit_us"]
    stamps = sorted(job["effect_committed_us"] for job in jobs)
    return dict(first_effect_us=stamps[0], last_effect_us=stamps[-1],
                first_from_commit_us=stamps[0] - origin, last_from_commit_us=stamps[-1] - origin,
                first_from_submit_us=stamps[0] - score["first_submit_us"],
                last_from_submit_us=stamps[-1] - score["first_submit_us"],
                first_16_from_commit_us=stamps[15] - origin)


def occupancy(jobs, start, stop):
    # Claimed timestamps are millisecond marks. The latest point consistent
    # with that mark and handler entry gives a conservative occupied duration.
    events = Counter()
    for job in jobs:
        claim = max(start, min(job["claimed_us"] + 999, job["entered_us"]))
        settled = job["settled_us"]
        assert claim <= settled
        events[claim] += 1
        events[settled] -= 1
    occupied = peak = 0
    duration = Counter()
    previous = start
    for when, delta in sorted(events.items()):
        duration[occupied] += when - previous
        occupied += delta
        assert 0 <= occupied <= SLOTS
        peak = max(peak, occupied)
        previous = when
    assert occupied == 0 and previous == stop
    return dict(peak_lower=peak, mean_lower=sum(n * span for n, span in duration.items()) / (stop - start),
                microseconds_by_occupied_seats_lower=dict(sorted(duration.items())))


def pg_cpu(frame, score, tls):
    before, after = frame["before"], frame["after"]
    start, stop = score["first_submit_us"], score["last_settlement_us"]
    assert before["begin_us"] <= before["end_us"] <= start < stop <= after["begin_us"] <= after["end_us"]
    first, last = before["rows"], after["rows"]
    pid = str(frame["postmaster"])
    assert frame["clock_ticks"] == 100 and pid in first
    assert first.keys() == last.keys() and {str(x["backend"]) for x in tls} <= first.keys()
    assert all(first[key][:2] == last[key][:2] and first[key][2] <= last[key][2] for key in first)
    assert all(first[key][0] == int(pid) for key in first.keys() - {pid})
    return dict(ms=10 * sum(last[key][2] - first[key][2] for key in first), pids=len(first),
                slack_us=start - before["begin_us"] + after["end_us"] - stop)


def check_overlay(meta, lane, candidate, expected_metadata):
    assert meta == dict(lane=lane, observer=False, baseline_body_sha256=digest(body(STOCK)),
                        body_sha256=digest(body(candidate)), metadata=expected_metadata)


def check_terminal(queue, jobs, writer, mode):
    grant, = [row for row in queue["queue_statements"] if GRANT in row["query"]]
    assert grant["calls"] == queue["admitted_queries"] > 0 and grant["rows"] >= JOBS
    assert "WITH ORDINALITY" in grant["query"]
    statements = {row["query"]: row["calls"] for row in queue["queue_statements"]}
    assert statements["BEGIN ISOLATION LEVEL READ COMMITTED"] == statements["COMMIT"] == grant["calls"]
    assert not statements.get("ROLLBACK")
    counts = Counter(job["settled_xid"] for job in jobs)
    widths = Counter(counts.values())
    assert len(counts) == queue["terminal_xids"]
    assert max(widths) == queue["largest_terminal_cohort"] <= 8

    def stats(prefix):
        rows = [row for row in queue["queue_statements"] if row["query"].startswith(prefix)]
        assert len(rows) <= 1
        return sum(x["calls"] for x in rows), sum(x["rows"] for x in rows)

    bulk, bulk_rows = stats(COHORT)
    scalar, scalar_rows = stats(TERMINAL)
    assert bulk == queue["completion_cohort_queries"] and bulk + scalar == len(counts)
    assert scalar == scalar_rows and bulk_rows + scalar_rows == JOBS
    if mode:
        assert writer["mode"] == mode and writer["confirmed_free_claims"] == grant["calls"]
        expected = {int(width): count for width, count in widths.items() if width > 1}
        assert sum(writer["cohort_submissions_by_width"][1:]) == bulk
        assert {width: count for width, count in enumerate(writer["cohort_submissions_by_width"], 1)
                if width > 1 and count} == expected
        assert 0 <= writer["saturated_flushes"] <= writer["refill_flushes"]
        if mode == "fixed8ms":
            assert writer["refill_flushes"] == 0
    else:
        assert writer is None
    return dict(grant_sql=grant["query"], grant_calls=grant["calls"], grant_rows=grant["rows"],
                cohort_calls=bulk, scalar_calls=scalar, terminal_xids=len(counts),
                terminal_xid_widths=dict(sorted(widths.items())))


def measure(folder, phase, lane, cfg, run, transfer, readiness):
    assert run.read_json(folder / "dispatch.json").get("exit_code") == 0
    spec = cfg["lanes"][lane]
    engine, inventory = spec["engine"], spec["inventory"]
    summary = run.score_arm(folder, dict(profile="short"), phase, engine, inventory)
    assert [summary["host_boot_id"], summary["host_kernel"], summary["pg_mount"]["maj:min"]] == cfg["host"]
    assert summary["completed"] == JOBS and summary["failed"] == 0
    summary.update(transfer.host_cpu(folder, summary, cfg["clock_ticks"]))
    start, commit, stop = (summary[key] for key in ("first_submit_us", "first_commit_us", "last_settlement_us"))
    summary.update(raw_sha256=run.digest(folder / "raw.tar.gz"), submit_us=stop-start, drain_us=stop-commit,
                   runner_cpu_ms=summary["runner_cpu_ms_per_job"] * JOBS,
                   host_cpu_ms=(summary["pinned_user_cpu_ms_per_job"] +
                                summary["pinned_kernel_cpu_ms_per_job"]) * JOBS)
    suite = cfg["product"]
    with tarfile.open(folder / "raw.tar.gz") as archive:
        names = archive.getnames()
        assert len(names) == len(set(names))
        def get(name):
            return run.file_json(archive, f"adapter/{name}.json")

        ledger = get("ledger")
        jobs = ledger["jobs"]
        summary.update(effects=effects(jobs, summary), stages=readiness.stage_times(ledger, summary))
        if engine != "mesh":
            summary["connections"] = readiness.connection_samples(archive, engine)
            if lane == "river-warm":
                summary["startup"] = readiness.startup(archive, "warm", summary, cfg["clock_ticks"])
            return summary

        assert ledger["phase"] == phase and ledger["local_smoke"] is False
        row, settings, details = map(get, ("row", "postgres-settings", "storage-topology"))
        assert get("build") == suite["build"] and ledger["database"] == settings
        assert row["runners"] == row["peak_processes"] == 1 and row["slots"] == SLOTS
        assert row["phase"] == phase and row["poll_ms"] == 250 and row["connection_cap"] == 20
        assert row["completed"] == row["reconciled_jobs"] == JOBS
        assert row["committed_work_records"] == 2 * JOBS and row["durable_enqueue_transactions"] == 1
        assert row["event_buffer_bytes"] is None and not row["events_dropped"] and not row["event_logs_dropped"]
        assert details["topology"] == "stock" and details["storage"] == "default"
        assert details["args_storage"] == "x" and details["work_mem"] == "4MB" and details["max_connections"] == 20

        broker, runner = get("process-identity")
        backend = inventory["engines"]["mesh"]
        executable, = [path for path in backend["verified_files"] if path.endswith("/mesh")]
        for record, verb in ((broker, "session"), (runner, "run")):
            transfer.child_identity(record, executable, suite["build"]["files"]["mesh"], verb, broker["parent_pid"])
        assert broker["pid"] != runner["pid"] and {job["worker_pid"] for job in jobs} == {runner["pid"]}
        transfer.check_affinity(get("process-affinity"), {runner["pid"]})
        assert transfer.text(archive, "adapter/broker-output.log").splitlines()[0] == "SESSION_READY"
        assert get("database") == dict(commit_timestamps=True, database_connection_limit=20, durable=True,
                                       max_connections=20, reserve=0, superuser_reserve=0, tls=True,
                                       user="parity_producer")
        origin, = ledger["transactions"]
        receipt = json.loads(transfer.text(archive, "adapter/enqueue-batch_000000.log"))
        assert start < commit <= origin["acknowledged_us"]
        assert receipt["count"] == JOBS and receipt["ids"] == origin["enqueued_ids"]
        assert receipt["xid"] == origin["xid"] and get("short-producer-batch") == {"calls": 1, "rows": JOBS}
        producer = get("producer-writer")
        assert producer["enabled"] and (producer["bulk"], producer["scalar"]) == (1, 0)

        tls = get("postgres-tls-sessions")
        roles = Counter(x["role"] for x in tls)
        assert all(x["tls"] for x in tls) and len(tls) == len({x["backend"] for x in tls})
        assert roles["compiled_caller"] == 8 and roles["parity_producer"] == 1
        assert 2 <= roles["parity_subject"] <= 4 and len(roles) == 3
        assert len(tls) <= row["peak_connections"] == summary["peak_connections"] <= 20
        writes = get("trusted-application-statements")
        for suffix in ("attempts", "effects"):
            statement, = (stmt for stmt in writes if stmt["query"].startswith("INSERT INTO public.mesh_jobs_parity_" + suffix))
            assert statement["calls"] == statement["rows"] == JOBS
        health = [stmt for stmt in writes if stmt["query"] == "SELECT $1"]
        probes = sum(stmt["calls"] for stmt in health)
        observed = get("raw")
        assert len(health) <= 1 and len(writes) == 2 + len(health)
        assert observed["idle_seconds"] >= 3 and observed["raw_idle_sql_calls"] > 0
        assert 0 <= probes < 256 and probes == sum(stmt["rows"] for stmt in health)
        assert observed["raw_application_sql_calls"] == 2 * JOBS + probes
        summary["connections"] = readiness.connection_samples(archive, engine)
        summary.update(checkout_probes=probes, tls_sessions=dict(roles))

        check_overlay(get("logical-overlay"), lane, cfg["candidate"], cfg["grant_metadata"])
        if lane == "proof-fixed":
            proof = get("grant-contract")
            assert proof["checks"] and all(proof["checks"].values())
            assert proof["overlay"] == {k: v for k, v in get("logical-overlay").items() if k not in ("lane", "observer")}
            summary["checked_grant_proof"] = proof
        else:
            assert "adapter/grant-contract.json" not in names

        runner_lines = transfer.text(archive, "adapter/runner-0.log").splitlines()
        mode = spec["mode"]
        assert runner_lines[0] == "READY" and len(runner_lines) == (3 if mode else 2)
        writer = json.loads(runner_lines[1].removeprefix("jobs_grouped_writer ")) if mode else None
        report = json.loads(runner_lines[-1])
        slots = report["queues"]["default"]
        assert report["drained"] and report["state"] == slots["state"] == "stopped"
        assert report["last_error"] is None and not any(report[k] for k in (
            "undrained", "recovered", "stale_acks", "pending_claims"))
        assert report["claimed"] == report["finished"] == report["outcomes"]["completed"] == JOBS
        assert not any(n for key, n in report["outcomes"].items() if key != "completed")
        assert slots["limit"] == SLOTS and slots["drained"] and slots["in_flight"] == slots["pending_claims"] == 0
        queue = get("queue-policy")
        assert queue["queue_policy"] == ledger["queue_policy"] == row["queue_policy"] == "success_cohort"
        summary.update(check_terminal(queue, jobs, writer, mode), writer=writer,
                       committed_occupancy=occupancy(jobs, commit, stop),
                       sampled_peak_executing=summary["peak_executing"])
        if lane == "proof-refill":
            assert writer["refill_flushes"] > 0, "actual dispatcher refill did not activate"
        if phase != "diagnostic":
            pg = pg_cpu(get("pg-cpu-bracket"), summary, tls)
            summary.update(pg_cpu_ms=pg["ms"], pg_cpu=pg)
    return summary


def gate(rows):
    a, b, primary, fixed = (rows[lane] for lane in ("immediate-a", "immediate-b", "refill", "fixed"))
    clocks = {}
    for key in CLOCKS:
        faster = min(a[key], b[key])
        saving = faster - primary[key]
        threshold = max(.05 * faster, abs(a[key] - b[key]))
        clocks[key] = dict(saving_us=saving, required_more_than_us=threshold, passed=saving > threshold,
                           fixed_minus_refill_us=fixed[key] - primary[key])
    checks = dict(
        earlier_last_effect=primary["effects"]["last_from_commit_us"] <
        min(row["effects"]["last_from_commit_us"] for row in (a, b)),
        real_refill=primary["writer"]["refill_flushes"] > 0,
        **{key: primary[key] <= max(a[key], b[key]) for key in CPU},
    )
    return dict(clocks=clocks, checks=checks, passed=all(x["passed"] for x in clocks.values()) and all(checks.values()))


def peer_pairs(groups, peers):
    def metrics(row):
        return {key: row[key] for key in (*CLOCKS, "runner_cpu_ms", "host_cpu_ms")} | {
            name: row["effects"][name] for name in ("first_from_commit_us", "last_from_commit_us")}

    pairs = {}
    for lane in peers:
        differences = [Counter(metrics(row["measured"][lane])) for row in groups]
        for weights, row in zip(differences, groups, strict=True):
            weights.subtract(metrics(row["measured"]["refill"]))
        pairs[lane] = {key: dict(groups=values, median_delta=median(values), min=min(values), max=max(values))
                       for key in differences[0]
                       for values in [[row[key] for row in differences]]}
    return pairs
