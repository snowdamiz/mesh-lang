"""Verify real cap-71 archives and derive both clocks from committed ledgers."""

from collections import Counter
from datetime import datetime, timezone
from functools import cache
import importlib.util
import json
from pathlib import Path
import re
import runpy
import sys
import tarfile

from adapter import CAP, INPUT

PACKET = Path(__file__).resolve().parent
SUITE = PACKET.parents[1]
ROOT = SUITE.parents[1]
PRIOR = PACKET.parent / "2026-11-23-ordinary-slow-runner-concentration"
TOPOLOGY = PACKET.parent / "2026-11-23-ordinary-runner-topology"
sys.path.insert(0, str(SUITE))
import run
from interleave import host_cpu
from verify import require, xid

JOBS, SLOTS = 512, 64
CPUS = [0, 1]
DBS = {"mesh": "mesh", "oban": "oban", "go": "river", "rust": "apalis"}
CONNECT = re.compile(r"^(?P<at>\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\.\d+) UTC \[(?P<pid>\d+)\] "
                     r".*? \| LOG:\s+(?P<event>connection authorized:|disconnection:)(?P<detail>.*)$")
AUTH = re.compile(r"\buser=(\S+) database=(\S+)(?: application_name=(\S+))?")


@cache
def reconciler():
    spec = importlib.util.spec_from_file_location("slow_cap71_verify", SUITE / "verify.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    require(module.PROFILES["slow"] == (1, 64, 68, INPUT, True), "canonical slow verifier drifted")
    module.PROFILES = {**module.PROFILES, "slow": (2, SLOTS, CAP, INPUT, True)}
    return module.reconcile


@cache
def mesh_verifier():
    # Bind only the cap and the isolated profile validator. The archived
    # verifier still receives the real, unmodified row, ledger and receipts.
    saved = sys.path[:]
    sys.path[:0] = [str(PRIOR), str(TOPOLOGY)]
    try:
        verifier = runpy.run_path(str(PRIOR / "replay.py"))["verify_arm"]
    finally:
        sys.path[:] = saved
    verifier.__globals__.update(CAP=CAP, reconciler=lambda runners: reconciler() if runners == 2 else None)
    return verifier


def hold_peak(jobs):
    events = [(j[name], change) for j in jobs
              for name, change in (("attempt_committed_us", 1), ("effect_committed_us", -1))]
    active = peak = 0
    for _, change in sorted(events):
        active += change
        peak = max(peak, active)
    return peak


def physical(log, engine, start, end):
    live, during, seen = {}, {}, set()
    whole_peak = window_peak = 0
    marks = Counter()
    previous = 0

    def observe():
        nonlocal window_peak
        require(all(db == DBS[engine] and user != "parity_owner" and tls
                    for user, db, _, tls in live.values()), "timed window contains an unexpected physical session")
        window_peak = max(window_peak, len(live))
        during.update(live)

    for line in log.splitlines():
        match = CONNECT.match(line)
        if not match:
            continue
        when = int(datetime.fromisoformat(match["at"]).replace(tzinfo=timezone.utc).timestamp() * 1_000_000)
        pid = int(match["pid"])
        if previous < start <= when:
            observe()
        previous = when
        if match["event"] == "connection authorized:":
            auth = AUTH.search(match["detail"])
            require(auth is not None and pid not in live, "connection authorization record is incomplete")
            user, db, app = auth.groups()
            live[pid] = (user, db, app or "", "SSL enabled" in match["detail"])
            seen.add(pid)
            marks["connect"] += 1
        else:
            require(pid in live, "disconnect has no successful authorization")
            del live[pid]
            marks["disconnect"] += 1
        if start <= when <= end:
            observe()
        whole_peak = max(whole_peak, len(live))
    roles = Counter(session[0] for session in during.values())
    names = Counter(session[2] for session in during.values())
    require(marks["connect"] == marks["disconnect"] and not live and whole_peak <= CAP
            and window_peak > 0 and len(seen) >= window_peak, "incomplete physical census or cap violation")
    if engine in ("mesh", "oban", "rust"):
        expected = 17 if engine == "mesh" else 69
        require(window_peak == expected and len(during) == expected and roles["parity_producer"] == 1,
                "expected warm physical roles changed")
    else:
        require(roles == {"parity_subject": len(during)} and names["river:producer_observer"] == 1
                and all(any(name.endswith(f":runner-{index}") for name in names) for index in range(2))
                and all(name == "river:producer_observer" or re.fullmatch(
                    r"river:parity_[a-f0-9]{16}:runner-[01]", name) for name in names),
                "River producer/observer sharing or physical runner names differ")
    return dict(peak=window_peak, whole_cluster_peak=whole_peak, unique_timed_backends=len(during),
                by_user=dict(roles), by_application_name=dict(names), event_counts=dict(marks))


def lifecycle(doc, engine):
    check = doc["post_arm"]
    require(check == {"cap": CAP, "dealloc": 0, "sessions": {"parity_owner": 1}},
            "counter eviction, lingering clients, or wrong cap in the post-arm census")
    statements = doc["statements"]
    shapes = {
        "mesh": ("from mesh_jobs.claim_batch_v1(", "mesh_jobs.settle_state(", ("mesh_jobs.recover(", "mesh_jobs.try_renew(")),
        "go": ("with locked_jobs as", "with job_input as", ("river_leader", "river_client", "with jobs_to_schedule")),
        "oban": ("for update skip locked", "completed_at", ("attempted_at", "oban_peers")),
        "rust": ("from apalis.get_jobs(", "done_at = now()", ("lock_at = now()", "last_seen", "reenqueue")),
    }
    claim, settle, maintenance = shapes[engine]
    groups = {name: [] for name in ("attempt", "effect", "claim", "settlement", "lease_or_maintenance")}
    for item in statements:
        q = " ".join(item["query"].lower().split())
        if q.startswith("insert into ") and "mesh_jobs_parity_attempts" in q:
            groups["attempt"].append(item)
        elif q.startswith("insert into ") and "mesh_jobs_parity_effects" in q:
            groups["effect"].append(item)
        elif item["role"] == "parity_subject":
            if claim in q:
                groups["claim"].append(item)
            elif settle in q and (engine != "oban" or q.startswith("update ") and "oban_jobs" in q):
                groups["settlement"].append(item)
            elif any(fragment in q for fragment in maintenance):
                groups["lease_or_maintenance"].append(item)
    totals = {key: {field: sum(row[field] for row in rows) for field in ("calls", "rows", "total_exec_time")}
              for key, rows in groups.items()}
    for key in ("attempt", "effect"):
        require(totals[key]["calls"] == totals[key]["rows"] == JOBS, f"{key} SQL does not match the ledger")
    require(totals["claim"]["calls"] > 0 and totals["settlement"]["calls"] > 0,
            f"{engine} did not witness claim and settlement SQL")
    return totals


def ownership(files, ledger, engine, postgres):
    jobs = ledger["jobs"]
    if engine in ("go", "rust"):
        name = "river/measured/ledger.json" if engine == "go" else "raw-writes.json"
        writes = files(name)
        required = {job["job_key"]: job["attempt_xid"] for job in jobs}
        actual = {item["job_key"]: item["xid"] for item in writes["attempts"]}
        require(actual == required and len(writes["attempts"]) == len(writes["effects"]) == JOBS,
                "peer ownership records do not match the application XIDs")
        owner = {item["job_key"]: item["worker_pid"] for item in writes["attempts"]}
    else:
        owner = {j["job_key"]: j["worker_pid"] for j in jobs}
    if engine == "go":
        config = files("river/config.json")
        require(config["profile_config"] == dict(runners=2, slots=64, physical_connection_cap=CAP,
                     input_sha256=INPUT, full_unmeasured_warmup=True) and not config["poll_only"]
                and config["visible_cpus"] == config["gomaxprocs"] == 2 and config["runner_pool_max"] == 68
                and config["producer_observer_pool_max"] == 1
                and config["submit_boundary"] == "before_batch_read", "River configuration or clock changed")
        roster = [files(f"river/measured/runner-{i}-capacity.json") for i in (0, 1)]
        require(all(0 < row["peak_concurrent_handlers"] <= 32 and row["handler_calls"] ==
                    sum(pid == row["pid"] for pid in owner.values()) for row in roster),
                "River launched, claimed or ran the wrong work")
        native = writes["jobs"]
        require(len(native) == JOBS and all(row["state"] == "completed" and not row["errors"]
                    and row["claimed"] < row["finalized_at"] for row in native),
                "River native running claim or terminal state changed")
    else:
        saved = files("runner-topology.json")
        roster = [dict(row, pid=int(pid)) for pid, row in saved.items()] if engine != "rust" else list(saved.values())
        pids = {entry["pid"] for entry in roster}
        if engine == "rust":
            runtime = files("runtime.json")
            require(runtime["cpu_affinity"] == CPUS and runtime["runner_tokio_threads"] == 2
                    and runtime["runner_pool_size"] == "slots + 2" and "lock_tasks + persist_results + LISTEN" in
                    runtime["queue_config"], "Apalis pool, lifecycle or CPU settings differ")
            for name in ("ready.json", "final-sessions.json"):
                status = files(name)
                by_app = Counter(row["application_name"] for row in status["sessions"] if row["ssl"])
                listeners = Counter(row["application_name"] for row in status["sessions"] if row["listener"])
                require(by_app == {"apalis:runner-0": 34, "apalis:runner-1": 34, "apalis:producer_observer": 1}
                        and listeners == {"apalis:runner-0": 1, "apalis:runner-1": 1}
                        and status["workers"] == ["runner-0", "runner-1"], "Apalis shared pool or listener drift")
            pid_for_lock = {name: row["pid"] for name, row in saved.items()}
            require(all(j["native_state"] == "Done" and 0 < j["last_lock_us"] <= j["settled_us"]
                        and pid_for_lock[j["lock_by"]] == owner[j["job_key"]] for j in jobs),
                    "Apalis persisted lock or runner ownership differs")
        else:
            masks = files("process-affinity.json")
            require(set(map(int, masks)) == {postgres["pid"], *pids}
                    and all(mask == CPUS for mask in masks.values()), "runner and PostgreSQL CPU affinity differ")
            if engine == "oban":
                reports = files("oban-emulator.json")
                require({report["pid"] for report in reports} == pids
                        and all(report["schedulers_online"] == 2 for report in reports),
                        "live Oban BEAM roster differs")
                sessions = files("postgres-tls-sessions.json")
                require(Counter(row["role"] for row in sessions if row["tls"]) ==
                        {"parity_subject": 68, "parity_producer": 1}, "Oban warm role allocation differs")
    pids = {row["pid"] for row in roster}
    require(len(roster) == len(pids) == 2 and all(row["slots"] == 32 for row in roster)
            and set(owner.values()) == pids, "ledger does not account for both 32-slot runners")
    for row in roster:
        own = [j for j in jobs if owner[j["job_key"]] == row["pid"]]
        row.update(jobs=len(own), hold_peak=hold_peak(own))
        require(row["hold_peak"] <= 32, "per-runner committed overlap exceeds 32")
    return roster


def score(folder, engine, phase, inventory, design):
    spec = inventory["engines"][engine]
    request = run.read_json(folder / "request.json")
    with tarfile.open(folder / "raw.tar.gz") as archive:
        def files(name):
            return run.file_json(archive, "adapter/" + name)

        def logs(name):
            member = archive.getmember("adapter/" + name)
            require(member.isfile(), "expected one archived regular log")
            with archive.extractfile(member) as stream:
                return stream.read().decode()

        host = run.file_json(archive, "host.json")
        invocation = run.file_json(archive, "invocation.json")
        expected = dict(phase=phase, profile="slow", engine=engine, attempt=request["attempt"])
        require(all(invocation[k] == request[k] == value for k, value in expected.items())
                and run.file_json(archive, "status.json")["exit_code"] == 0
                and host["verified_files"] == spec["verified_files"] and host["affinity"] == CPUS
                and [host["boot_id"], host["kernel"], host["pg_mount"]["maj:min"]] == design["host"],
                "native status, file pins, host or invocation differ")
        ledger = files("ledger.json")
        txn, = ledger["transactions"]
        jobs = ledger["jobs"]
        artifact = {key: spec[key] for key in ("version", "artifact_sha256")}
        row = reconciler()(ledger, engine, "slow", artifact)
        require(ledger.get("local_smoke", False) is False and ledger.get("phase", phase) == phase
                and row["completed"] == JOBS and all(job["hold_ms"] == 100 for job in jobs),
                "native slow ledger changed")
        if engine == "mesh":
            mesh = mesh_verifier()(files, logs, "two", dict(artifact, executable_sha256=design["mesh_elf"]),
                                   local=False, phase=phase)
            row["mesh_bulk_and_slots"] = {name: mesh[name] for name in (
                "bulk_calls", "bulk_rows", "scalar_calls", "scalar_rows", "used_app_backends",
                "unoccupied_before_last_claim_ms", "last_hold_end_commit_ms")}
        elif engine == "go":
            require(txn["submitted_us"] <= txn["read_finished_us"] <= txn["decoded_us"] <=
                    txn["enqueue_called_us"] <= txn["committed_us"] <= txn["returned_us"],
                    "River native submit boundary differs")
        else:
            returned = txn["returned_us"] if engine == "rust" else txn["acknowledged_us"]
            require(txn["submitted_us"] <= txn["committed_us"] <= returned, "native producer window differs")
        app = {xid(j[k], k) for j in jobs for k in ("attempt_xid", "effect_xid")}
        terminal = {xid(j["settled_xid"], "settled_xid") for j in jobs}
        require(len(app) == 2 * JOBS and app.isdisjoint(terminal) and xid(txn["xid"], "producer") not in app | terminal
                and (engine in ("go", "rust") or len(terminal) == JOBS), "commit transaction identities differ")
        postgres = files("postgres-affinity.json")
        require(postgres["cpus"] == CPUS and type(postgres["pid"]) is int, "PostgreSQL CPU pin differs")
        peak = hold_peak(jobs)
        require(0 < peak <= SLOTS, "global committed hold overlap exceeds the configured slots")
        census = physical(logs("physical-postgres.log"), engine, txn["submitted_us"], row["last_settlement_us"])
        require(census["whole_cluster_peak"] >= ledger["peak_connections"], "ledger peak not supported by physical census")
        row.update(physical=census, lifecycle=lifecycle(files("sql-lifecycle.json"), engine),
                   ownership=ownership(files, ledger, engine, postgres), committed_hold_peak=peak,
                   write_xids=len(app), terminal_xids=len(terminal),
                   submit_ms=(row["last_settlement_us"]-row["first_submit_us"])/1000,
                   drain_ms=(row["last_settlement_us"]-row["first_commit_us"])/1000)
        with archive.extractfile("host-samples.jsonl") as stream:
            row.update(run.sample_window(host, [json.loads(line) for line in stream], row))
    row.update(host_cpu(folder, row, design["clock_ticks"]))
    return dict(row, raw_sha256=run.digest(folder / "raw.tar.gz"))
