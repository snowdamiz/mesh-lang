"""Offline verification adapted from the released factorial at 776dcaee3."""
from collections import Counter, defaultdict
import json
from pathlib import Path
import tarfile

import run
from interleave import host_cpu
from verify import PROFILES, reconcile, require, xid
from witness import logger_off

SUITE = Path(run.__file__).resolve().parent
CPUS = [0, 1]
JOBS = 1024
ROLES = dict(compiled_caller=32, parity_subject=32, parity_producer=1)
HOST = ["90ec1c24-08c8-4bc1-aa28-79b6db1bd13a", "6.8.0-149-generic", "8:0"]
INPUT = PROFILES["noop"][3]


def text(archive, name):
    member = archive.getmember(name)
    require(member.isfile(), f"invalid archive member {name}")
    return archive.extractfile(member).read().decode()


def check_affinity(masks, worker_pids):
    require(len(masks) == len(worker_pids) + 1 and worker_pids <= set(map(int, masks))
            and all(mask == CPUS for mask in masks.values()), "runner or PostgreSQL affinity differs")


def child_identity(record, file, pin, verb, adapter):
    require(record["argv"] == [str(file), verb] and record["executable_sha256"] == pin
            and record["cpus"] == CPUS and record["parent_pid"] == adapter
            and record["launch_start_ticks"] == record["start_ticks_before"] == record["start_ticks_after"] > 0,
            "direct process identity differs")
    require((record["method"], record["observed_executable"], record["exe_errno"])
            in (("proc-exe", str(file), None), ("owned-launch", None, 13)),
            "unexpected kernel executable fallback")


def cpu(proof, start, stop):
    a, b = proof["before"], proof["after"]
    assert a["begin_us"] <= a["end_us"] <= start < stop <= b["begin_us"] <= b["end_us"]
    assert proof["clock_ticks"] == 100
    pid = str(proof["postmaster"])
    first, last = a["rows"], b["rows"]
    assert all(v[0] == int(pid) for r in (first, last) for k, v in r.items() if k != pid)
    paired = [k for k in first.keys() & last.keys() if first[k][1] == last[k][1]]
    assert pid in paired and len(paired) >= 40, "PostgreSQL work escaped the stable CPU bracket"
    ticks = [last[k][2] - first[k][2] for k in paired]
    assert min(ticks) >= 0
    return dict(ms=sum(ticks)*10, paired=len(paired), unmatched=sorted((first.keys() | last.keys())-set(paired)),
                slack_us=start-a["begin_us"]+b["end_us"]-stop)


def occupancy(jobs):
    edges = defaultdict(Counter)
    xids = defaultdict(set)
    for job in jobs:
        pid = job["worker_pid"]
        edges[pid][job["claimed_us"]] += 1
        edges[pid][job["settled_us"]] -= 1
        xids[job["settled_xid"]].add(pid)
    peaks = {}
    for pid, timeline in {**edges, "total": sum_edges(edges.values())}.items():
        occupied = peak = 0
        for stamp in sorted(timeline):
            occupied += timeline[stamp]
            require(occupied >= 0, "negative database occupancy")
            peak = max(peak, occupied)
        require(occupied == 0 and peak <= (64 if pid == "total" else 8), "database slot cap")
        peaks[str(pid)] = peak
    require(all(len(pids) == 1 for pids in xids.values()), "terminal XID crossed runner")
    return peaks


def sum_edges(timelines):
    combined = Counter()
    for timeline in timelines:
        combined.update(timeline)
    return combined


def score_arm(folder, phase, lane, inventory, expected_build, expected_routes):
    path = folder / "raw.tar.gz"
    spec = inventory["engines"]["mesh"]
    req = run.read_json(folder / "request.json")
    require(run.read_json(folder / "dispatch.json").get("exit_code") == 0, "native dispatch failed")
    with tarfile.open(path) as raw:
        def get(name):
            return run.file_json(raw, f"adapter/{name}.json")

        require(len(raw.getnames()) == len(set(raw.getnames())), "duplicate archive entry")
        invocation = run.file_json(raw, "invocation.json")
        require(all(invocation[k] == req[k] == expected for k, expected in
                    dict(profile="noop", phase=phase, engine="mesh", attempt=req["attempt"]).items())
                and run.file_json(raw, "status.json")["exit_code"] == 0, "arm failed or wrong invocation")
        host = run.file_json(raw, "host.json")
        require([host["boot_id"], host["kernel"], host["pg_mount"]["maj:min"]] == HOST
                and host["affinity"] == CPUS and len(host["available_cpus"]) == 4
                and host["verified_files"] == spec["verified_files"], "host, affinity or files changed")
        build, ledger, row, settings, details, queue = map(get, (
            "build", "ledger", "row", "postgres-settings", "storage-topology", "queue-policy"))
        require(build == expected_build and ledger["phase"] == row["phase"] == phase
                and ledger["local_smoke"] is row["local_smoke"] is False
                and ledger["database"] == settings and settings["version"] == "17.11"
                and ledger["slots"] == row["slots"] == 64
                and ledger["runners"] == row["runners"] == row["peak_processes"] == 8
                and row["poll_ms"] == 250 and row["connection_cap"] == 96
                and row["completed"] == row["reconciled_jobs"] == JOBS
                and row["committed_work_records"] == 2 * JOBS
                and row["durable_enqueue_transactions"] == 1, "scope, durability or product artifact changed")
        require(details["topology"] == "stock" and details["storage"] == "default"
                and details["work_mem"] == "4MB" and details["args_storage"] == "x"
                and details["max_connections"] == 96, "product storage or topology drift")
        score = reconcile(ledger, "mesh", "noop",
                                        {key: spec[key] for key in ("version", "artifact_sha256")})
        origin, = ledger["transactions"]
        require(origin["submitted_us"] < origin["committed_us"] <= origin["acknowledged_us"],
                "submit or broker boundary changed")
        require(get("database") == dict(commit_timestamps=True, database_connection_limit=96, durable=True,
                                        max_connections=96, reserve=0, superuser_reserve=0,
                                        tls=True, user="parity_producer"), "broker/database proof changed")

        broker, *runners = get("process-identity")
        executable, = (file for file, pin in spec["verified_files"].items()
                       if file.endswith("/mesh") and pin == build["files"]["mesh"])
        launch_parent = broker["parent_pid"]
        for record, verb in [(broker, "session"), *((record, "run") for record in runners)]:
            child_identity(record, executable, build["files"]["mesh"], verb, launch_parent)
        pids = {runner["pid"] for runner in runners}
        require(len(runners) == len(pids) == 8 and broker["pid"] not in pids, "wrong runner/broker roster")
        check_affinity(get("process-affinity"), pids)
        ready = get("canonical-ready")
        tls = ready["sessions"]
        roles = Counter(item["role"] for item in tls)
        require(roles == ROLES and len(tls) == len({item["backend"] for item in tls})
                and all(item["tls"] is True for item in tls)
                and len(tls) == row["peak_connections"] == score["peak_connections"] == 65
                and ready["lane"] == lane and ready["workers"] == {str(pid): ["2", "2", "4"] for pid in pids},
                "canonical TLS roster, scheduler supply or physical cap drifted")
        standard = {(item["backend"], item["role"], item["tls"]) for item in get("postgres-tls-sessions")}
        require(standard <= {(item["backend"], item["role"], True) for item in tls},
                "warm backend identities changed before submit")
        jobs = ledger["jobs"]
        by_pid = Counter(job["worker_pid"] for job in jobs)
        require(by_pid.keys() == pids, "compiled handlers did not execute in every runner")
        for index, runner in enumerate(runners):
            report = json.loads(text(raw, f"adapter/runner-{index}.log").splitlines()[-1])
            slots = report["queues"]["default"]
            require(report["drained"] and report["state"] == slots["state"] == "stopped"
                    and report["last_error"] is None
                    and not any(report[k] for k in ("undrained", "recovered", "stale_acks", "pending_claims"))
                    and report["claimed"] == report["finished"] == report["outcomes"]["completed"] == by_pid[runner["pid"]]
                    and all(n == 0 for key, n in report["outcomes"].items() if key != "completed")
                    and slots["limit"] == 8 and slots["drained"] and slots["in_flight"] == slots["pending_claims"] == 0,
                    "runner exceeded its slot cap or failed to drain")

        cohorts = Counter(xid(item["settled_xid"], "terminal") for item in jobs)
        effects = {xid(job[key], key) for job in jobs for key in ("attempt_xid", "effect_xid")}
        require(len(effects) == 2 * JOBS and not effects & cohorts.keys()
                and xid(origin["xid"], "enqueue") not in effects | cohorts.keys(),
                "application, enqueue or terminal transaction overlaps")
        grouped = lane != "checked-individual"
        require(0 < row["peak_threads"] <= (72 if grouped else 64), "runner threads exceed original supply")
        policy = "success_cohort" if grouped else "set_based"
        route = "claim_batch_bounded_v1" if grouped else "claim_batch_set_v1"
        def calls(function):
            return sum(stmt["calls"] for stmt in queue["queue_statements"]
                       if f"FROM mesh_jobs.{function}(" in stmt["query"])
        require(queue["queue_policy"] == ledger["queue_policy"] == row["queue_policy"] == policy
                and calls(route) == queue["admitted_queries"] > 0
                and calls("claim_batch_set_v1" if route != "claim_batch_set_v1"
                          else "claim_batch_bounded_v1") == 0
                and len(cohorts) == queue["terminal_xids"]
                and max(cohorts.values()) == queue["largest_terminal_cohort"]
                and (1 < max(cohorts.values()) <= 8 if grouped else len(cohorts) == JOBS),
                "terminal XID contract or checked grant route changed")
        logs = [text(raw, f"adapter/runner-{i}.log") for i in range(8)]
        witness = logger_off(jobs, queue, logs, [runner["pid"] for runner in runners],
                             get("raw")["namespace"], grouped)
        require(get("public-grants") == expected_routes
                and "adapter/logical-overlay.json" not in raw.getnames(), "public grant definitions differ")
        require(get("raw")["case_sha256"] == INPUT and get("raw")["idle_seconds"] >= 3,
                "input or warm readiness differs")
        enqueue = get("producer-writer")
        require(enqueue["enabled"] and (enqueue["bulk"], enqueue["scalar"]) == (8, 0),
                "product dense enqueue route changed")
        statements = get("trusted-application-statements")
        for suffix in ("attempts", "effects"):
            writes, = (stmt for stmt in statements
                       if stmt["query"].startswith("INSERT INTO public.mesh_jobs_parity_" + suffix))
            require(writes["calls"] == writes["rows"] == JOBS, "application commit counts changed")
        health = [stmt for stmt in statements if stmt["query"] == "SELECT $1"]
        probes = sum(stmt["calls"] for stmt in health)
        require(len(health) <= 1 and len(statements) == 2 + len(health)
                and probes == sum(stmt["rows"] for stmt in health)
                and get("raw")["raw_application_sql_calls"] == 2 * JOBS + probes
                and (probes == 2 * JOBS if lane.endswith("-cold") else 0 <= probes < 32),
                "actual checkout policy did not match the assigned lane")
        with raw.extractfile("host-samples.jsonl") as stream:
            score.update(run.sample_window(host, [json.loads(line) for line in stream], score))
        pg = cpu(get("pg-cpu-bracket"), score["first_submit_us"], score["last_settlement_us"])
        require(not pg["unmatched"], "unmatched PostgreSQL PID in CPU window")
    score.update(host_cpu(folder, score, 100))
    start, commit, stop = (score[key] for key in ("first_submit_us", "first_commit_us", "last_settlement_us"))
    return dict(score, raw_sha256=run.digest(path), checkout_probes=probes, physical_tls_by_role=dict(roles),
                submit_us=stop-start, drain_us=stop-commit,
                last_effect_us=max(row["effect_committed_us"] for row in jobs)-commit,
                terminal_xids=len(cohorts), terminal_widths=dict(Counter(cohorts.values())), terminal_witness=witness,
                occupancy=occupancy(jobs), pg_cpu=pg, pg_cpu_ms=pg["ms"],
                runner_cpu_ms=score["runner_cpu_ms_per_job"]*JOBS, peak_threads=row["peak_threads"],
                host_cpu_ms=(score["pinned_user_cpu_ms_per_job"]+score["pinned_kernel_cpu_ms_per_job"])*JOBS)
