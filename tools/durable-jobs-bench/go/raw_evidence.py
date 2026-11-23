"""Verify River's raw exports before translating them to matrix evidence."""

from collections import Counter
import datetime
from functools import cache
import hashlib
import json
from pathlib import Path
import re

from prepare_input import harness
from verify import PROFILES, database_check, positive_int, require, xid

HERE = Path(__file__).resolve().parent
EPOCH = datetime.datetime(1970, 1, 1, tzinfo=datetime.timezone.utc)
FAULTS = [
    {"request_id": "rollback", "rollback": True,
     "jobs": [{"key": "rollback", "payload": "fixture", "hold_ms": 0, "fail_first": False}]},
    {"request_id": "retry", "rollback": False,
     "jobs": [{"key": "retry", "payload": "fixture", "hold_ms": 1, "fail_first": True}]},
]


def unique_pairs(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, f"duplicate JSON field: {key}")
        result[key] = value
    return result


def read_json(path):
    return json.loads(path.read_text(), object_pairs_hook=unique_pairs)


def read_lines(path):
    return [json.loads(line, object_pairs_hook=unique_pairs) for line in path.read_text().splitlines()]


def epoch_us(timestamp):
    date = datetime.datetime.fromisoformat(timestamp)
    require(date.tzinfo is not None, "missing evidence timestamp timezone")
    delta = date - EPOCH
    return (delta.days * 86400 + delta.seconds) * 1_000_000 + delta.microseconds


def sha256(data):
    return hashlib.sha256(data).hexdigest()


@cache
def frozen_input(profile):
    _, slots, _, digest, _ = PROFILES[profile]
    settings = read_json(harness["CONFIG"])
    batches = harness["inputs"](settings["workloads"][profile], slots, settings["seed"])
    hashes = {batch["request_id"] + ".json": sha256(
        (json.dumps(batch, indent=2, sort_keys=True, allow_nan=False) + "\n").encode())
        for batch in batches}
    require(sha256("".join(hashes.values()).encode()) == digest, "frozen River input drifted")
    return batches, {"input_sha256": digest, "batch_digests": hashes}


def build_identity(config, binary_hash):
    info = config["build"]
    module = (HERE / "go.mod").read_text()
    go_version = re.search(r"^go (\S+)$", module, re.M)[1]
    versions = dict(re.findall(r"^[ \t]*(\S+)[ \t]+(v\S+)(?:[ \t]+// indirect)?$", module, re.M))
    sums = {(name, version): checksum for name, version, checksum in (
        line.split() for line in (HERE / "go.sum").read_text().splitlines())}
    pins = {name: {"Path": name, "Version": version, "Sum": sums[name, version]}
            for name, version in versions.items()}
    deps = info["Deps"]
    require(info["GoVersion"] == "go" + go_version and len(deps) == len(pins)
            and {dep["Path"]: dep for dep in deps} == pins, "River build versions or module sums differ from pins")
    require(config["binary_sha256"] == binary_hash, "executed River artifact differs from binary digest")
    return (f"river {versions['github.com/riverqueue/river']} / "
            f"pgx {versions['github.com/jackc/pgx/v5']} / {info['GoVersion']}")


def index(rows, key, label):
    indexed = {key(row): row for row in rows or []}
    require(len(indexed) == len(rows or []), f"duplicate {label} identity")
    return indexed


def verify_trial(path, profile, batches):
    runners, slots, cap, digest, _ = PROFILES[profile]
    schema = read_json(path / "schema.json")
    require(re.fullmatch(r"parity_[a-f0-9]{16}", schema), "invalid River schema identity")
    raw = read_json(path / "ledger.json")
    receipts = read_lines(path / "enqueue.jsonl")
    require([row["request_id"] for row in receipts] == [b["request_id"] for b in batches],
            "wrong producer receipt sequence")
    expected = [job for b in batches if not b["rollback"] for job in b["jobs"]]
    require([row["key"] for row in raw["jobs"]] == [job["key"] for job in expected],
            "persisted jobs differ from ordered frozen input")
    attempt_counts = {job["key"]: 1 + job["fail_first"] for job in expected}
    attempts = index(raw["attempts"], lambda row: (row["job_key"], row["attempt"]), "attempt")
    effects = index(raw["effects"], lambda row: row["job_key"], "effect")
    products = index(raw["product"], lambda row: row["request_id"], "product")
    require(attempts.keys() == {(key, n) for key, count in attempt_counts.items() for n in range(1, count + 1)},
            "missing or substituted attempt row")
    require(effects.keys() == attempt_counts.keys(), "missing or substituted effect row")
    require(products.keys() == {b["request_id"] for b in batches if not b["rollback"]},
            "missing or substituted product row, or a rollback leaked")
    separate_xids = set()

    def independent(identity):
        require(identity not in separate_xids, "reused producer or application transaction XID")
        separate_xids.add(identity)
        return identity

    def app_identity(row):
        require(row["engine"] == "river" and row["namespace"] == schema, "wrong application row scope")
        return xid(row["xid"], "application xid")

    owners = {}
    transactions = []
    prior_id, prior_return = 0, 0
    for batch, row in zip(batches, receipts):
        request = batch["request_id"]
        require(row["rollback"] is batch["rollback"] and row["ids"].keys()
                == {job["key"] for job in batch["jobs"]}, f"{request}: wrong receipt keys or rollback status")
        for job in batch["jobs"]:
            encoded_id = row["ids"][job["key"]]
            require(re.fullmatch(r"[1-9][0-9]*", encoded_id), "invalid queue ID")
            queue_id = int(encoded_id)
            require(queue_id > prior_id, f"{request}: reused or out-of-order queue ID")
            prior_id = queue_id
        identity = independent(xid(row["xid"], "producer xid") % 2**32)
        submitted = positive_int(row["submitted_us"], "submitted_us")
        returned = positive_int(row["returned_us"], "returned_us")
        require(prior_return <= submitted <= returned, f"{request}: invalid producer timestamps")
        prior_return = returned
        if batch["rollback"]:
            require(row["commit"] is None, f"{request}: rollback has a commit timestamp")
            continue
        commit = epoch_us(row["commit"])
        product = products[request]
        require(submitted <= commit <= returned and app_identity(product) == identity
                and epoch_us(product["committed"]) == commit, f"{request}: product did not commit with enqueue")
        transaction = dict(request_id=request, submitted_us=submitted, committed_us=commit,
                           returned_us=returned, xid=row["xid"], enqueued_ids=row["ids"])
        transactions.append(transaction)
        for job in batch["jobs"]:
            owners[job["key"]] = transaction

    jobs = []
    settlements = {}
    attempts_by_pid = Counter()
    for expected_job, row in zip(expected, raw["jobs"]):
        key = row["key"]
        count = attempt_counts[key]
        owner = owners[key]
        committed = owner["committed_us"]
        require((row["id"], row["payload_sha256"], row["hold_ms"], row["fail_first"])
                == (owner["enqueued_ids"][key], sha256(expected_job["payload"].encode()),
                    expected_job["hold_ms"], expected_job["fail_first"])
                and type(row["hold_ms"]) is int and type(row["fail_first"]) is bool,
                f"{key}: wrong job ID or arguments")
        require(row["state"] == "completed" and type(row["attempt"]) is int and row["attempt"] == count
                and type(row["max_attempts"]) is int and row["max_attempts"] == 3
                and row["kind"] == row["queue"] == "parity", f"{key}: wrong terminal job")
        errors = row["errors"] or []
        require([(e["attempt"], e["error"]) for e in errors] == (
            [(1, "fixture_fail_first")] if count == 2 else []), f"{key}: wrong retry history")
        settled = epoch_us(row["settled"])
        settled_xid = xid(row["settled_xid"], "terminal xid")
        require(settlements.setdefault(settled_xid, settled) == settled,
                f"{key}: batched terminal XID has inconsistent commit timestamps")
        claimed = epoch_us(row["claimed"])
        finalized = epoch_us(row["finalized_at"])
        attempt_rows = [attempts[key, n] for n in range(1, count + 1)]
        effect = effects[key]
        effect_identity = independent(app_identity(effect))
        effect_commit = epoch_us(effect["committed"])
        require(effect["effect_key"] == key and epoch_us(effect["effected_at"]) <= effect_commit + 5_000,
                f"{key}: substituted effect identity or invalid time")
        started = []
        for attempt in attempt_rows:
            independent(app_identity(attempt))
            entered = epoch_us(attempt["entered_at"])
            written = epoch_us(attempt["committed"])
            require(attempt["effect_key"] == key and type(attempt["attempt"]) is int
                    and committed < written < settled and committed <= entered + 5_000
                    and entered <= written + 5_000, f"{key}: wrong attempt identity or commit window")
            started.append((entered, written))
            attempts_by_pid[positive_int(attempt["worker_pid"], "worker_pid")] += 1
        entered, first_write = started[0]
        require(first_write < effect_commit < settled
                and committed <= claimed + 5_000 and claimed <= started[-1][0] + 5_000
                and started[-1][1] <= finalized + 5_000 and finalized <= settled + 5_000,
                f"{key}: invalid claim, attempt, effect or terminal ordering")
        if count == 2:
            require(effect_commit < started[1][1], f"{key}: effect did not survive the retry")
        else:
            require(effect_commit - first_write >= (row["hold_ms"] - 1) * 1000,
                    f"{key}: requested hold not observed")
        jobs.append(dict(id=row["id"], job_key=key, payload_sha256=row["payload_sha256"],
                         hold_ms=row["hold_ms"], fail_first=row["fail_first"], state=row["state"],
                         attempt=row["attempt"], attempts=len(attempt_rows), effects=int(effect is not None),
                         entered_us=entered,
                         attempt_xid=attempt_rows[0]["xid"], effect_xid=str(effect_identity),
                         settled_xid=row["settled_xid"], attempt_committed_us=first_write,
                         effect_committed_us=effect_commit, settled_us=settled, claimed_us=claimed))
    require(separate_xids.isdisjoint(settlements), "terminal XID reused by producer or application writes")
    if profile == "sparse" and batches != FAULTS:
        for previous, current in zip(jobs, transactions[1:]):
            require(current["submitted_us"] - previous["settled_us"] >= 249_000,
                    "sparse arrival did not wait for durable settlement and the 250 ms gap")

    peak_handlers = 0
    require(len(list(path.glob("runner-*-capacity.json"))) == runners, "wrong number of runner capacity receipts")
    seen_pids = set()
    for n, allocation in enumerate(harness["allocate_slots"](runners, slots)):
        worker = read_json(path / f"runner-{n}-capacity.json")
        pid = positive_int(worker["pid"], "runner PID")
        peak, calls = worker["peak_concurrent_handlers"], worker["handler_calls"]
        require(pid not in seen_pids and type(peak) is int and type(calls) is int
                and worker["slots"] == allocation and calls == attempts_by_pid.pop(pid, 0)
                and 0 <= peak <= min(calls, allocation) and bool(peak) == bool(calls),
                f"runner {n}: slot cap or handler calls did not reconcile")
        seen_pids.add(pid)
        peak_handlers += peak
    require(not attempts_by_pid, "attempt attributed to an unknown runner")

    samples = read_lines(path / "samples.jsonl")
    require(samples and all(s["all_connections_tls"] is True and s["processes"] == runners
                            and 0 < s["connections"] <= cap for s in samples)
            and samples[-1]["terminal"] == len(jobs), "sampled TLS, connection cap or settlement mismatch")
    peaks = {k: max(s[k] for s in samples) for k in ("connections", "database_running_rows", "rss_bytes")}
    metrics = read_json(path / "metrics.json")
    summary = dict(completed=len(jobs), attempt_rows=len(attempts), effect_rows=len(effects),
                   producer_commits=len(products), rolled_back_batches=len(receipts) - len(products),
                   retried=sum(n - 1 for n in attempt_counts.values()),
                   terminal_transactions=len(settlements),
                   largest_terminal_batch=max(Counter(j["settled_xid"] for j in jobs).values()),
                   peak_handlers=peak_handlers, slot_cap=slots, physical_cap=cap, **peaks)
    require(all(metrics[k] == summary[k] for k in ("completed", "rolled_back_batches", "retried"))
            and metrics["concurrent_handler_upper_bound"] == peak_handlers
            and all(metrics["peak_samples"][k] == v for k, v in peaks.items())
            and metrics.get("input_sha256") == (None if batches == FAULTS else digest),
            "raw River metrics differ from reconciled evidence")
    return jobs, transactions, metrics, summary


def verify_run(raw, profile, binary_hash, *, smoke, warmup=False):
    config = read_json(raw / "config.json")
    version = build_identity(config, binary_hash)
    runners, slots, cap, digest, full_warm = PROFILES[profile]
    require(config["engine"] == "river" and config["profile"] == profile and config["smoke"] is smoke
            and config["profile_config"] == dict(runners=runners, slots=slots, physical_connection_cap=cap,
                                                 input_sha256=digest, full_unmeasured_warmup=full_warm),
            "wrong River runner profile")
    require(read_json(raw / "status.json")["ok"] is True, "River run did not finish successfully")
    database = read_json(raw / "database.json")
    version_num = positive_int(database["server_version_num"], "PostgreSQL version number")
    database["version"] = f"{version_num // 10000}.{version_num % 10000}"
    database_check(database, cap)
    require(database["other_client_backends"] == 0, "unexpected database clients at preflight")

    batches, _ = frozen_input(profile)
    jobs, transactions, metrics, measured = verify_trial(raw / "measured", profile, batches)
    summary = {"measured": measured}
    if smoke:
        summary["faults"] = verify_trial(raw / "faults", profile, FAULTS)[3]
    if warmup:
        require(full_warm, "unexpected warmup trial")
        summary["warmup"] = verify_trial(raw / "warmup", profile, batches)[3]
    ledger = dict(engine="go", version=version, profile=profile, artifact_sha256=binary_hash,
                  input_sha256=digest, runners=runners, slots=slots, database=database,
                  runner_cpu_seconds=metrics["runner_cpu_seconds_including_startup_and_idle"],
                  peak_executing=measured["peak_handlers"], peak_connections=measured["connections"],
                  peak_runner_rss_bytes=measured["rss_bytes"], jobs=jobs, transactions=transactions)
    return ledger, summary
