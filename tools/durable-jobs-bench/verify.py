"""Reconcile native Jobs evidence with the frozen input and compute common windows."""

from functools import cache
import hashlib
import importlib.util
import json
import math
from pathlib import Path
from statistics import median


SUITE = Path(__file__).resolve().parent
ROOT = SUITE.parents[1]
ENGINES = ("mesh", "oban", "go", "rust")
# Runners, total slots, PostgreSQL max_connections, frozen batch digest, warmup.
PROFILES = {
    "noop": (8, 64, 96, "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a", True),
    "slow": (1, 64, 68, "42bb9e2be94c17b36110081430faf2647a04f3a3e0f2decf00e7c216fed462fd", True),
    "sparse": (1, 1, 5, "20225faf6274696987b2192136f344d69be243f0a21dc9afad71e714c6ee08c5", False),
    "short": (1, 16, 20, "ac0bcbf2465ea6d6807e8bfcdcfc3b1dacfa18b775e3f4c074257d66bfbba9d1", False),
    "large": (8, 64, 96, "9d7f130f2cc690487582f73c9bea5fbb1f2661f8e416beb2f1574a6f868cc5c1", True),
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def plan(kind):
    """Five profile rotations and a balanced four-engine crossover."""
    profiles = tuple(PROFILES)
    blocks = []
    groups = range(5) if kind == "baseline" else range(2)
    for group in groups:
        for position in range(len(profiles)):
            index = (group + position) % len(profiles)
            profile = profiles[index]
            if kind == "baseline":
                order = [ENGINES[(offset + group + index) % 4] for offset in (0, 1, 3, 2)]
            else:
                order = list(ENGINES) if group == 0 else list(reversed(ENGINES))
            phases = []
            if PROFILES[profile][-1] and kind != "diagnostic":
                phases.append("warmup")
            phases.append({"baseline": "measured", "controls": "control", "diagnostic": "diagnostic"}[kind])
            blocks.append({"group": group + 1, "profile": profile,
                           "order": order, "phases": phases})
    return blocks


@cache
def expected_inputs(profile):
    spec = importlib.util.spec_from_file_location("jobs_parity", ROOT / "scripts/jobs-parity.py")
    parity = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(parity)
    settings = json.loads(parity.CONFIG.read_text())
    _, slots, _, expected_hash, _ = PROFILES[profile]
    batches = parity.inputs(settings["workloads"][profile], slots, settings["seed"])
    hashes = []
    requests = {}
    for batch in batches:
        encoded = (json.dumps(batch, indent=2, sort_keys=True, allow_nan=False) + "\n").encode()
        hashes.append(hashlib.sha256(encoded).hexdigest())
        requests[batch["request_id"]] = {
            job["key"]: (hashlib.sha256(job["payload"].encode()).hexdigest(),
                         job["hold_ms"], job["fail_first"]) for job in batch["jobs"]
        }
    require(hashlib.sha256("".join(hashes).encode()).hexdigest() == expected_hash,
            f"{profile}: frozen input has drifted")
    return requests


def positive_int(value, name):
    require(type(value) is int and value > 0, f"invalid {name}")
    return value


def xid(value, name):
    require(isinstance(value, str) and value.isascii() and value.isdecimal() and int(value) > 2,
            f"invalid {name}")
    return int(value)


def database_check(database, cap):
    require(database["version"] == "17.11", "expected PostgreSQL 17.11")
    for flag in ("tls", "fsync", "synchronous_commit", "full_page_writes", "track_commit_timestamp"):
        require(database[flag] is True, f"PostgreSQL {flag} disabled or unverified")
    for field, value in {"max_connections": cap, "database_connection_limit": cap,
                         "reserved_connections": 0, "superuser_reserved_connections": 0}.items():
        require(type(database[field]) is int and database[field] == value, f"wrong PostgreSQL {field}")


def reconcile(ledger, engine, profile, artifact):
    runners, slots, cap, digest, _ = PROFILES[profile]
    for field, value in {"engine": engine, "profile": profile, "input_sha256": digest,
                         "runners": runners, "slots": slots, **artifact}.items():
        require(ledger[field] == value, f"unexpected {field}")
    database_check(ledger["database"], cap)
    require(type(ledger["peak_executing"]) is int and 0 <= ledger["peak_executing"] <= slots,
            "invalid observed slot occupancy")
    require(positive_int(ledger["peak_connections"], "peak_connections") <= cap,
            "connection peak exceeded physical cap")
    cpu = ledger["runner_cpu_seconds"]
    require(type(cpu) in (float, int) and math.isfinite(cpu) and cpu > 0, "invalid runner CPU")
    positive_int(ledger["peak_runner_rss_bytes"], "peak runner RSS")
    expected = expected_inputs(profile)
    transactions = ledger["transactions"]
    require([row["request_id"] for row in transactions] == list(expected), "unexpected producer transactions")
    durable = {}
    transaction_ids = set()
    previous_submit = None
    for transaction in transactions:
        request = transaction["request_id"]
        enqueued = transaction["enqueued_ids"]
        require(enqueued.keys() == expected[request].keys(), f"{request}: incorrect enqueue keys")
        identity = xid(transaction["xid"], "producer xid")
        require(identity not in transaction_ids, "producer receipts share a transaction")
        transaction_ids.add(identity)
        submitted = positive_int(transaction["submitted_us"], "submitted_us")
        committed = positive_int(transaction["committed_us"], "committed_us")
        require(submitted <= committed and (previous_submit is None or submitted >= previous_submit),
                "producer submission and commit are out of order")
        previous_submit = submitted
        for key, queue_id in enqueued.items():
            require(isinstance(queue_id, str) and queue_id, f"{key}: missing durable queue ID")
            durable[key] = (queue_id, *expected[request][key], transaction)
    require(len({row[0] for row in durable.values()}) == len(durable), "reused queue ID")

    seen = set()
    stages = {name: [] for name in ("commit_to_pickup", "submit_to_pickup", "commit_to_settlement",
                                    "submit_to_settlement", "entry_to_attempt_commit",
                                    "attempt_to_effect_commit", "effect_commit_to_settlement")}
    claims = []
    settled = []
    for row in ledger["jobs"]:
        key = row["job_key"]
        require(key in durable and key not in seen, f"{key}: unexpected or repeated persisted job")
        seen.add(key)
        queue_id, payload, hold, fail, transaction = durable[key]
        require((row["id"], row["payload_sha256"], row["hold_ms"], row["fail_first"])
                == (queue_id, payload, hold, fail), f"{key}: job does not match frozen input and receipt")
        require(type(row["fail_first"]) is bool and type(row["hold_ms"]) is int,
                f"{key}: invalid persisted arguments")
        require(row["state"] == "completed" and all(type(row[field]) is int and row[field] == 1
                for field in ("attempt", "attempts", "effects")), f"{key}: incorrect terminal state or write counts")
        first = xid(row["attempt_xid"], "attempt_xid")
        second = xid(row["effect_xid"], "effect_xid")
        for work in (first, second):
            require(work not in transaction_ids, f"{key}: reused application transaction XID")
            transaction_ids.add(work)
        if "settled_xid" in row:
            require(xid(row["settled_xid"], "settled_xid") not in (first, second),
                    f"{key}: terminal state shares an application transaction")
        entered, first_us, second_us, settled_us = [positive_int(row[field], field) for field in (
            "entered_us", "attempt_committed_us", "effect_committed_us", "settled_us")]
        committed = transaction["committed_us"]
        submitted = transaction["submitted_us"]
        # Runner timestamps in the frozen fixture use up to millisecond precision.
        require(submitted <= entered + 5000 and committed <= entered + 5000 and entered <= first_us + 5000
                and committed <= first_us < second_us <= settled_us,
                f"{key}: work is outside the durable producer→terminal window")
        require(second_us - first_us >= (hold - 1) * 1000, f"{key}: requested hold not observed")
        if "claimed_us" in row:
            claimed = positive_int(row["claimed_us"], "claimed_us")
            require(committed <= claimed + 5000 and claimed <= entered + 5000,
                    f"{key}: invalid claim ordering")
            claims.append(claimed)
        settled.append(settled_us)
        for stage, duration in zip(stages.values(), (entered - committed, entered - submitted,
                settled_us - committed, settled_us - submitted, first_us - entered,
                second_us - first_us, settled_us - second_us), strict=True):
            stage.append(max(0, duration) / 1000)
    require(seen == durable.keys(), "missing durable jobs")
    first_submit = min(row["submitted_us"] for row in transactions)
    first_commit = min(row["committed_us"] for row in transactions)
    last_settlement = max(settled)
    if profile == "sparse":
        by_key = {row["job_key"]: row for row in ledger["jobs"]}
        for previous, current in zip(transactions, transactions[1:]):
            previous_job = by_key[next(iter(previous["enqueued_ids"]))]
            require(current["submitted_us"] - previous_job["settled_us"] >= 249000,
                    "sparse arrival did not wait for settlement and the 250 ms gap")
    require(last_settlement > first_commit, "nonpositive durable window")
    return {
        "completed": len(seen), "failed": 0, "ordered_independent_writes": len(seen),
        "first_submit_us": first_submit, "first_commit_us": first_commit,
        "last_settlement_us": last_settlement,
        "submit_jobs_per_s": len(seen) * 1_000_000 / (last_settlement - first_submit),
        "commit_jobs_per_s": len(seen) * 1_000_000 / (last_settlement - first_commit),
        "first_claim_from_commit_ms": (min(claims) - first_commit) / 1000 if len(claims) == len(seen) else None,
        "claim_to_last_settlement_ms": (last_settlement - min(claims)) / 1000 if len(claims) == len(seen) else None,
        "runner_cpu_ms_per_job": cpu * 1000 / len(seen),
        "peak_runner_rss_bytes": ledger["peak_runner_rss_bytes"],
        "peak_executing": ledger["peak_executing"], "peak_connections": ledger["peak_connections"],
        **{f"{key}_{statistic}_ms": fn(values) for key, values in stages.items()
           for statistic, fn in (("p50", median), ("p99", lambda s: sorted(s)[math.ceil(len(s)*0.99)-1]))},
    }
