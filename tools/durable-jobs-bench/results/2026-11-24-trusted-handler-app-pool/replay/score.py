"""Replay closed native arms, including the live handler markers against SQL commits."""
from collections import Counter
import json
from statistics import median
import tarfile

import run
from interleave import host_cpu
from qualification import check_affinity, diagnostic_reconciler, mesh_helpers, text
from verify import reconcile, require, xid

MODES = dict(v1="v1", **{"service-a": "service", "service-b": "service", "direct": "direct",
                         "service-observed": "service-observed", "direct-observed": "direct-observed"})
GEOMETRY = {"noop": (2, 32, 16, 96, 1024), "short": (1, 16, 8, 20, 128)}
FIELDS = ("entered_us", "lookup_begin_us", "lookup_end_us", "first_return_us", "second_return_us")
STAGES = ("entry_to_lookup_ms", "live_lookup_ms", "lookup_to_first_return_ms",
          "first_to_second_return_ms", "entry_to_first_return_ms")


def distribution(values):
    return dict(n=len(values), median=median(values), min=min(values), max=max(values))


def handler_witness(lines_by_pid, jobs, mode):
    observed = mode.endswith("-observed")
    by_id = {job["id"]: job for job in jobs}
    witnesses = {}
    for pid, lines in lines_by_pid.items():
        require(lines.count("APP_POOL_REGISTRATION_OK") == int(mode == "service-observed"),
                "missing or unsolicited registered-owner identity and synchronous reply")
        for line in lines:
            if not line.startswith("APP_POOL_JOB "):
                continue
            require(observed, "observer-off arm emitted a job timestamp")
            witness = json.loads(line.removeprefix("APP_POOL_JOB "))
            job_id = witness["job_id"]
            require(job_id not in witnesses and job_id in by_id, "duplicate or unexpected handler witness")
            job = by_id[job_id]
            require((witness["job_key"], witness["attempt"], witness["entered_us"], pid)
                    == (job["job_key"], job["attempt"], job["entered_us"], job["worker_pid"]),
                    "handler witness identity differs from the committed attempt row")
            stamps = [witness[field] for field in FIELDS]
            require(all(type(stamp) is int and stamp > 0 for stamp in stamps) and stamps == sorted(stamps),
                    "handler clock moved backwards")
            entered, begin, end, first, second = stamps
            require(end <= job["attempt_committed_us"] <= first <= job["effect_committed_us"] <= second
                    <= job["settled_us"], "handler timestamp boundaries disagree with durable SQL and exit")
            witnesses[job_id] = [span / 1000 for span in
                                 (begin - entered, end - begin, first - end, second - first, first - entered)]
    if not observed:
        return {}
    require(witnesses.keys() == by_id.keys(), "missing completed handler marker")
    return {stage: distribution([interval[position] for interval in witnesses.values()])
            for position, stage in enumerate(STAGES)}


def score_arm(folder, block, phase, lane, inventory, host_pin, clock_ticks):
    profile = block["profile"]
    mode = MODES[lane]
    runners, slots, app_lanes, cap, count = GEOMETRY[profile]
    spec = inventory["engines"]["mesh"]
    artifact = {field: spec[field] for field in ("artifact_sha256", "version")}

    with tarfile.open(folder / "raw.tar.gz", "r:gz") as archive:
        request = run.read_json(folder / "request.json")
        invocation = run.file_json(archive, "invocation.json")
        for field, value in dict(profile=profile, phase=phase, engine="mesh", attempt=request["attempt"]).items():
            require(invocation[field] == request[field] == value, f"unexpected {field}")
        require(run.file_json(archive, "status.json")["exit_code"] == 0, "failed or interrupted adapter")
        host = run.file_json(archive, "host.json")
        require([host["boot_id"], host["kernel"], host["pg_mount"]["maj:min"]] == host_pin
                and host["architecture"] == "x86_64" and host["affinity"] == [0, 1]
                and len(host["available_cpus"]) == 4 and host["verified_files"] == spec["verified_files"],
                "staged host, resources, or artifacts changed")

        def files(name):
            return run.file_json(archive, "adapter/" + name)

        ledger, row, raw, runner, order = map(files, ("ledger.json", "row.json", "raw.json",
                                                       "runner-identity.json", "ordered-writes.json"))
        for evidence in (row, ledger):
            require(evidence["phase"] == phase and evidence["local_smoke"] is False
                    and evidence["execution_contract"] == "ordinary-trusted-shared-process", "mixed execution contract")
        for field, value in artifact.items():
            require(row[field] == value, "reported runner provenance differs")
        reconcile_profile = diagnostic_reconciler(runners) if profile == "noop" else reconcile
        score = reconcile_profile(ledger, "mesh", profile, artifact)
        require(score["completed"] == count == order["jobs"] == order["ordered"], "incomplete ordered writes")
        require(order["worker_pids"] <= runners and len(ledger["transactions"]) == 1, "changed V1 inputs")
        require(ledger["database"] == files("postgres-settings.json") and row["connection_cap"] == cap,
                "database settings changed")
        jobs = ledger["jobs"]
        roster = [entry["pid"] for entry in runner["runners"]]
        exe = spec["argv"][3] + "/mesh"
        require(runner["hashes"] == {"mesh": spec["verified_files"][exe],
                                     "mesh.json": spec["verified_files"][exe + ".json"]}
                and all(entry["executable"] == exe for entry in runner["runners"])
                and runner["build"]["files"]["mesh"] == runner["hashes"]["mesh"]
                and runner["mode"] == mode and len(set(roster)) == len(roster) == runners,
                "executing runner differs from the pinned lane")
        require(runner["slots_per_runner"] == slots and runner["app_sessions_per_runner"] == app_lanes
                and runner["postgres_cap"] == cap and runner["queues_per_runner"] == 1
                and runner["scheduler_min"] == runner["scheduler_max"] == 2
                and runner["batch"] == runner["listen"] == "1", "runner geometry changed")
        check_affinity(files("process-affinity.json"), set(roster))
        caps = {"parity_subject": runners * 4, "parity_producer": 1, "compiled_caller": runners * app_lanes}
        capacity = files("sql-capacity.json")
        namespace = raw["namespace"]
        require(capacity["role_caps"] == caps and capacity["sql_queues"] == [
            dict(name="default", state="active", namespace=namespace, concurrency=runners * slots)
        ] and row["peak_connections"] == ledger["peak_connections"] == sum(caps.values()),
                "PostgreSQL observed connection peak, role or SQL queue cap changed")
        broker = files("database.json")
        transaction = ledger["transactions"][0]
        require(text(archive, "adapter/broker-output.log").splitlines()[0] == "SESSION_READY"
                and broker["user"] == "parity_producer" and broker["tls"] is broker["durable"] is True
                and broker["max_connections"] == broker["database_connection_limit"] == cap
                and transaction["submitted_us"] < transaction["committed_us"] <= transaction["acknowledged_us"],
                "warm broker or endpoint changed")
        statements = files("scalar-enqueues.json")
        require(len(statements) == 1 and all(q.startswith("SELECT mesh_jobs.enqueue(") and calls == count
                                              for q, calls in statements.items()),
                "producer did not execute the exact scalar enqueue count")
        helpers = mesh_helpers()
        used = helpers["verify_sessions"](files, caps, transaction["submitted_us"], set())
        score["app_sql_ms"] = helpers["verify_app"](files, jobs, namespace)
        require(raw["raw_application_sql_calls"] == 4 * count, "driver calls changed")
        terminal = {xid(job["settled_xid"], "terminal") for job in jobs}
        application = {xid(job[key], key) for job in jobs for key in ("attempt_xid", "effect_xid")}
        require(len(terminal | application | {xid(transaction["xid"], "producer")}) == 3 * count + 1,
                "application, producer and terminal SQL lost transaction independence")
        per_pid = Counter(job["worker_pid"] for job in jobs)
        require(set(per_pid) <= set(roster), "job outside runner roster")
        lines_by_pid = {}
        for index, pid in enumerate(roster):
            lines = text(archive, f"adapter/runner-{index}.log").splitlines()
            lines_by_pid[pid] = lines
            report = json.loads(lines[-1])
            require(lines.count("READY") == 1 and report["namespace"] == namespace and report["state"] == "stopped"
                    and report["drained"] and report["last_error"] is None
                    and not any(report[key] for key in ("undrained", "recovered", "stale_acks", "pending_claims"))
                    and report["claimed"] == report["finished"] == report["outcomes"]["completed"] == per_pid[pid]
                    and not any(n for key, n in report["outcomes"].items() if key != "completed")
                    and report["queues"]["default"]["limit"] == slots
                    and helpers["intervals"]([job for job in jobs if job["worker_pid"] == pid]) <= slots,
                    "runner did not complete and drain within its slot cap")
        score["handler_observer"] = handler_witness(lines_by_pid, jobs, mode)
        with archive.extractfile("host-samples.jsonl") as raw_samples:
            score.update(run.sample_window(host, [json.loads(line) for line in raw_samples], score))
    score.update(host_cpu(folder, score, clock_ticks))
    score.update(used_app_sessions=len(used), warmed_app_sessions=caps["compiled_caller"],
                 app_sql_calls=4 * count, terminal_transactions=len(terminal), jobs_by_runner=dict(per_pid),
                 drain_ms=(score["last_settlement_us"] - score["first_commit_us"]) / 1000,
                 submit_ms=(score["last_settlement_us"] - score["first_submit_us"]) / 1000,
                 raw_sha256=run.digest(folder / "raw.tar.gz"))
    return score


def evaluate(blocks):
    metrics = ("drain_ms", "submit_ms", "entry_to_attempt_commit_p50_ms",
               "entry_to_attempt_commit_p99_ms")
    out = {}
    for profile in GEOMETRY:
        pairs = []
        for item in blocks:
            if item["profile"] != profile:
                continue
            lanes = item["measured"]
            paired = {}
            for metric in metrics:
                a, b, direct, service_obs, direct_obs = (lanes[name][metric] for name in (
                    "service-a", "service-b", "direct", "service-observed", "direct-observed"))
                paired[metric] = dict(gain=min(a, b) - direct, duplicate_spread=abs(a-b),
                                      observer_service_tax_range=sorted([service_obs - a, service_obs - b]),
                                      observer_direct_tax=direct_obs - direct)
            paired["observed_lookup"] = {name: lanes[name]["handler_observer"]["live_lookup_ms"]
                                         for name in ("service-observed", "direct-observed")}
            paired["group"] = item["group"]
            pairs.append(paired)
        if pairs:
            gains = {key: distribution([pair[key]["gain"] for pair in pairs]) for key in metrics}
            decisive = sum(all(pair[key]["gain"] > pair[key]["duplicate_spread"]
                               for key in metrics[:3]) for pair in pairs)
            out[profile] = dict(pairs=pairs, faster_duplicate_gains=gains,
                                pairs_exceeding_duplicate_spread=decisive,
                                threshold_pass=len(pairs) == 6 and decisive >= 5
                                and (profile != "noop" or gains["drain_ms"]["median"] >= 62.4))
    return out
