"""Verify and score real slow ledgers with an explicit per-arm topology."""

from collections import Counter
from functools import cache
import gzip
import importlib.util
import json
from pathlib import Path
import re
import runpy
from statistics import median
import sys

from slow_arm import FROZEN, HERE, INPUT, LAYOUTS, PROFILE, TOPOLOGY

SUITE = HERE.parents[1]
sys.path.insert(0, str(SUITE))
from verify import PROFILES, require, xid

JOBS = 512
SLOTS = 64
CAP = 68
CONTRACT = "ordinary-trusted-shared-process"
EDGES = ("claimed_us", "entered_us", "attempt_committed_us", "effect_committed_us", "settled_us")
STAGES = ("claim_to_entry", "entry_to_first_write", "between_writes", "second_write_to_terminal")


@cache
def reconciler(runners):
    """Use isolated verifier state and leave the real ledger untouched."""
    require(runners in {layout[0] for layout in LAYOUTS.values()}, "unsupported slow topology")
    spec = importlib.util.spec_from_file_location("slow_topology_verify", SUITE / "verify.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.PROFILES = {**module.PROFILES, PROFILE: (runners, *PROFILES[PROFILE][1:])}
    return module.reconcile


@cache
def helpers():
    return runpy.run_path(str(TOPOLOGY / "replay.py"))


def phases(jobs, txn):
    origin = txn["committed_us"]
    last_claim = max(job["claimed_us"] for job in jobs)
    terminal = max(job["settled_us"] for job in jobs)
    require(all(origin <= job["claimed_us"] and
                [job[k] for k in EDGES] == sorted(job[k] for k in EDGES) for job in jobs),
            "claim, handler, application, or terminal phase ordering differs from original V1 ledger")

    def vacancy(end, padding):
        busy = sum(max(0, min(job["settled_us"], end) - min(job["claimed_us"] + padding, job["entered_us"]))
                   for job in jobs)
        return (end - origin - busy / SLOTS) / 1000

    output = dict(
        submit_ms=(terminal - txn["submitted_us"]) / 1000,
        drain_ms=(terminal - origin) / 1000,
        last_claim_ms=(last_claim - origin) / 1000,
        unoccupied_before_last_claim_ms=vacancy(last_claim, 0),
        unoccupied_before_last_claim_upper_ms=vacancy(last_claim, 1000),
        unoccupied_to_drain_ms=vacancy(terminal, 0),
        unoccupied_to_drain_upper_ms=vacancy(terminal, 1000),
    )
    for field, name in zip(EDGES[1:], ("entry", "hold_start_commit", "hold_end_commit", "terminal_commit"), strict=True):
        values = [(job[field] - origin) / 1000 for job in jobs]
        output[f"first_{name}_ms"] = min(values)
        output[f"last_{name}_ms"] = max(values)
    for name, left, right in zip(STAGES, EDGES[:-1], EDGES[1:], strict=True):
        output[f"normalized_{name}_ms"] = sum(job[right] - job[left] for job in jobs) / (SLOTS * 1000)
    require(abs(output["drain_ms"] - output["unoccupied_to_drain_ms"] -
                sum(output[f"normalized_{name}_ms"] for name in STAGES)) < 1e-8,
            "slot residence and vacancy do not account for the complete drain")
    return output


def verify_writer(files):
    before, end = (files(f"statements-{boundary}.json") for boundary in ("before", "end"))

    def delta(prefix):
        def totals(rows):
            matches = [row for row in rows if row["role"] == "parity_producer" and row["query"].startswith(prefix)]
            return [sum(row[key] for row in matches) for key in ("calls", "rows", "wal_bytes", "total_exec_time")]
        return [b - a for a, b in zip(totals(before), totals(end), strict=True)]

    bulk = delta("SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(")
    scalar = delta("SELECT mesh_jobs.enqueue(")
    require(bulk[:2] == [8, 504] and scalar[:2] == [8, 8] and bulk[2] > 0 and scalar[2] > 0,
            "true-bulk writes, WAL or ordered scalar remainder count drifted")
    return dict(bulk_calls=bulk[0], bulk_rows=bulk[1], scalar_calls=scalar[0], scalar_rows=scalar[1],
                producer_sql_ms=bulk[3] + scalar[3], bulk_wal_bytes=bulk[2], scalar_wal_bytes=scalar[2])


def verify_wire(log):
    batch = json.loads(gzip.decompress((FROZEN / "canonical-batch.json.gz").read_bytes()))
    expected = [j["key"] for j in batch["jobs"]]
    inputs = {j["key"]: j for j in batch["jobs"]}
    windows = []
    lines = log.splitlines()
    for index, command in enumerate(lines):
        bulk = "SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(" in command
        scalar = "SELECT mesh_jobs.enqueue(" in command
        if "LOG:  execute " not in command or not (bulk or scalar):
            continue
        detail = next((line for line in lines[index + 1:] if not line.startswith("\t")), "")
        require("DETAIL:  Parameters: " in detail, "missing producer wire parameters")
        if bulk:
            data = re.search(r"\$2 = '(.*)'$", detail)
            require(data is not None, "missing bulk wire parameter")
            specs = json.loads(data[1].replace("''", "'"))
            keys = [row["spec"]["args"]["key"] for row in specs]
            require(len(keys) == 63 and keys == [row["request_key"] for row in specs]
                    and [row["spec"]["args"] for row in specs] == [inputs[key] for key in keys],
                    "unexpected count, identity or payload in bulk positions")
            windows.append(dict(mode="bulk", keys=keys))
        else:
            data = re.search(r"\$2 = '(.*)', \$3 = '(.*?)', \$4 = ", detail)
            require(data is not None, "missing scalar wire parameter")
            args = json.loads(data[1].replace("''", "'"))["args"]
            key = args["key"]
            require(key == data[2] and args == inputs[key], "scalar work payload and request key differ")
            windows.append(dict(mode="scalar", keys=[key]))
    require([w["mode"] for w in windows] == ["bulk"] * 8 + ["scalar"] * 8
            and [key for window in windows for key in window["keys"]] == expected,
            "wire statement order or per-position job keys differ")
    return windows


def verify_arm(files, logs, label, artifact, *, local=True, phase="diagnostic", wire=False):
    runners, slots, apps = LAYOUTS[label]
    build, row, raw, ledger, setup, broker = map(files, (
        "build.json", "row.json", "raw.json", "ledger.json", "topology.json", "database.json"))
    require(build["files"]["mesh"] == artifact["executable_sha256"]
            and build["version"] == row["version"] == artifact["version"]
            and row["runners"] == ledger["runners"] == runners
            and row["slots"] == SLOTS and row["connection_cap"] == CAP
            and row["profile"] == PROFILE and row["input_sha256"] == raw["case_sha256"] == INPUT
            and row["execution_contract"] == ledger["execution_contract"] == build["execution_contract"] == CONTRACT
            and ledger["database"] == files("postgres-settings.json")
            and row["phase"] == ledger["phase"] == phase
            and row["local_smoke"] is ledger["local_smoke"] is local,
            "true-bulk V1 build, execution contract, or receipt identity changed")
    pids = setup["runner_pids_by_index"]
    require(len(pids) == len(set(pids)) == runners and setup["runners"] == runners
            and setup["slots_per_runner"] == slots and setup["app_sessions_per_runner"] == apps
            and setup["scheduler_min"] == setup["scheduler_max"] == 2 and setup["label"] == label
            and setup.get("wire", False) is wire, "unexpected runner allocation")
    if not local:
        masks = files("process-affinity.json")
        require(len(masks) == runners + 1 and set(pids) <= set(map(int, masks))
                and all(mask == [0, 1] for mask in masks.values()), "native runner or PostgreSQL affinity differs")
    caps = {"parity_subject": 4 * runners, "compiled_caller": 8, "parity_producer": 1}
    capacity = files("sql-capacity.json")
    namespace = raw["namespace"]
    require(capacity["role_caps"] == caps
            and capacity["sql_queues"] == [dict(name="default", state="active", namespace=namespace, concurrency=SLOTS)]
            and row["peak_connections"] == ledger["peak_connections"] == sum(caps.values())
            and row["peak_processes"] == runners,
            "SQL queue concurrency, role limits or process count differ")
    require(logs("broker-output.log").splitlines()[0] == "SESSION_READY"
            and broker["user"] == "parity_producer" and broker["tls"] is broker["durable"] is True
            and broker["max_connections"] == broker["database_connection_limit"] == CAP,
            "missing verified durable shared broker")
    samples = raw["samples"] + raw["idle_samples"]
    end = files("physical-end.json")
    physical = max(sample["physical_clients"] for sample in samples)
    require(all(sample["connections"] == sample["physical_clients"] == sum(caps.values()) for sample in samples)
            and physical <= CAP and end == dict(physical_clients=physical, by_role=caps)
            and raw["samples"][-1]["terminal"] == JOBS,
            "physical client cap or observer terminal count differs")
    score = reconciler(runners)(ledger, "mesh", PROFILE, {
        key: artifact[key] for key in ("version", "artifact_sha256")})
    jobs = ledger["jobs"]
    txn, = ledger["transactions"]
    require(len(jobs) == JOBS and txn["submitted_us"] < txn["committed_us"] <= txn["acknowledged_us"],
            "incomplete job ledger or invalid producer phase")
    evidence = helpers()
    used = evidence["verify_sessions"](files, caps, txn["submitted_us"], set())
    app_sql = evidence["verify_app"](files, jobs, namespace)
    app_xids = {xid(job[key], key) for job in jobs for key in ("attempt_xid", "effect_xid")}
    terminal_xids = {xid(job["settled_xid"], "terminal") for job in jobs}
    require(len(app_xids | terminal_xids | {xid(txn["xid"], "producer")}) == JOBS * 3 + 1,
            "producer, application and terminal XIDs are not independent")

    def hold_peak(rows):
        return evidence["intervals"]([dict(claimed_us=j["attempt_committed_us"],
                                           settled_us=j["effect_committed_us"]) for j in rows])

    require({j["worker_pid"] for j in jobs} <= set(pids), "unknown compiled handler PID")
    ownership = []
    for index, pid in enumerate(pids):
        owned = [j for j in jobs if j["worker_pid"] == pid]
        report = json.loads(logs(f"runner-{index}.log").splitlines()[-1])
        queue = report["queues"]["default"]
        peak = evidence["intervals"](owned)
        require(report["namespace"] == namespace and report["drained"] and report["last_error"] is None
                and not any(report[k] for k in ("undrained", "recovered", "stale_acks", "pending_claims"))
                and report["claimed"] == report["finished"] == report["outcomes"]["completed"] == len(owned)
                and all(count == 0 for name, count in report["outcomes"].items() if name != "completed")
                and queue["limit"] == slots and queue["in_flight"] == queue["pending_claims"] == 0
                and hold_peak(owned) <= peak <= slots, "local ownership, slot cap or clean drain differs")
        ownership.append(dict(pid=pid, jobs=len(owned), occupied_peak=peak, committed_hold_peak=hold_peak(owned),
                              contention_hints=report["batch_contention_hints"]))
    peak = evidence["intervals"](jobs)
    sampled = max(sample["executing"] for sample in samples)
    require(sampled == ledger["peak_executing"] <= SLOTS and hold_peak(jobs) <= peak <= SLOTS,
            "global occupancy exceeded cap")
    counters = [files(f"counters-{index}.json") for index in (1, 2, 3)]
    require(len({item["dealloc"] for item in counters}) == 1
            and counters[1]["runner"] - counters[0]["runner"] == raw["raw_idle_sql_calls"],
            "statement counter eviction or idle accounting mismatch")
    for field, name in (("runner", "runner_sql"), ("enqueue", "enqueue_sql"),
                        ("application", "application_sql"), ("claims", "claim")):
        require(counters[2][field] - counters[1][field] == raw[f"raw_{name}_calls"], f"{field} counter mismatch")
    require(raw["raw_application_sql_calls"] == 4 * JOBS
            and raw["runner_cpu_seconds_including_startup_and_idle"] == ledger["runner_cpu_seconds"],
            "application statements or CPU accounting differ")
    return dict(**score, **phases(jobs, txn), **verify_writer(files), layout=LAYOUTS[label],
                schedule_workers=2 * runners, warm_roles=caps, physical_clients_peak=physical,
                connection_cap_violations=0, slot_cap_violations=0, used_app_backends=used,
                application_xids=len(app_xids), terminal_xids=len(terminal_xids), enqueue_xids=1,
                runner_ownership=ownership, claim_to_terminal_peak=peak, committed_hold_peak=hold_peak(jobs),
                claim_calls=raw["raw_claim_calls"], queue_calls=raw["raw_runner_sql_calls"],
                hints=sum(item["contention_hints"] for item in ownership), application_sql_ms=app_sql,
                application_backends_used_count=len(used), producer_ack_ms=(txn["acknowledged_us"]-txn["submitted_us"])/1000,
                **({"positions": verify_wire(logs("postgres.log"))} if wire else {}))


def directory_arm(path, label, wire=False):
    from arm import read
    build = read(path / "build.json")
    pins = read(FROZEN / "local-pins.json")
    require(build["files"] == pins["fixture_files"], "wrong retained Darwin binary manifest")
    artifact = dict(version="3828f20d2d6beb683868cce33e6dd1958365914b",
                    artifact_sha256="65ae886ca2f48b993a0f1d30a0a46fd9abeaf5b18e33bc1c4748e1c570f24e51",
                    executable_sha256=pins["executable_sha256"])
    return verify_arm(lambda name: read(path / name), lambda name: (path / name).read_text(),
                      label, artifact, wire=wire)


def distribution(values):
    return dict(median=median(values), min=min(values), max=max(values))


def summarize(groups, candidate):
    metrics = ("submit_ms", "drain_ms", "runner_cpu_ms_per_job", "unoccupied_before_last_claim_ms",
               "first_hold_start_commit_ms", "last_hold_start_commit_ms", "first_hold_end_commit_ms",
               "last_hold_end_commit_ms", "last_claim_ms", "application_backends_used_count",
               "claim_calls", "queue_calls", "hints", "producer_sql_ms", "producer_ack_ms",
               "application_sql_ms", "committed_hold_peak",
               *(f"normalized_{name}_ms" for name in STAGES))
    labels = ("one-a", "one-b", candidate)
    paired = {}
    for key in metrics:
        comparisons = []
        for group in groups:
            arms = group["arms"]
            a, b, c = (arms[label][key] for label in labels)
            comparisons.append(dict(faster_control_minus_candidate=min(a, b) - c,
                                    control_spread=abs(a-b), candidate_minus_slower_control=c-max(a, b)))
        paired[key] = dict(by_group=comparisons, **{
            name: distribution([row[name] for row in comparisons]) for name in comparisons[0]})
    return dict(groups=groups, by_label={label: {key: distribution([g["arms"][label][key] for g in groups])
                                               for key in metrics} for label in labels}, paired=paired)


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", type=Path)
    parser.add_argument("label", choices=LAYOUTS)
    parser.add_argument("--wire", action="store_true")
    args = parser.parse_args()
    print(json.dumps(directory_arm(args.path, args.label, args.wire), sort_keys=True, indent=2))
