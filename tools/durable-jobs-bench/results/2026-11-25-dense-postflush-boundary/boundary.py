"""Validate backend evidence and join full XIDs within one private cluster."""
from collections import defaultdict
import json
import re
from statistics import median
import time

XID_RANGE = 1 << 32
CAPACITY = 4096
MAX_BRACKET_NS = 100_000
MAX_OFFSET_CHANGE_NS = 100_000
# Both existing endpoints retain microseconds. The hook retains nanoseconds.
US = 1000


def require(condition, message):
    if not condition:
        raise ValueError(message)


def clock_bracket():
    def sample():
        before = time.monotonic_ns()
        real = time.time_ns()
        return before, real, time.monotonic_ns()

    return min((sample() for _ in range(8)), key=lambda row: row[2] - row[0])


def validate_clock(start, end):
    a, b = start["clock"], end["clock"]
    for low, real, high in (a, b):
        require(0 < low <= high and 0 < real and high - low <= MAX_BRACKET_NS, "wide clock sample")
    require(a[2] < b[0] and start["after_ns"] < end["before_ns"], "reversed arm bracket")
    require(abs((a[1] - a[0]) - (b[1] - b[0])) <= MAX_OFFSET_CHANGE_NS, "wall clock offset changed")
    for item in (start, end):
        require(item["clock"][1] <= item["before_ns"] <= item["sql_us"] * US + US - 1
                and item["sql_us"] * US <= item["after_ns"], "PostgreSQL clock escaped client bracket")
    require(start["system_identifier"] == end["system_identifier"] and int(start["system_identifier"]) > 0,
            "cluster identity changed")
    bounds = (int(start["xid"]), int(end["xid"]))
    require(3 <= bounds[0] < bounds[1] < 1 << 64 and bounds[1] - bounds[0] < XID_RANGE,
            "ambiguous full XID bracket")
    return bounds


def lift_xid(value, bounds):
    require(type(value) is str and re.fullmatch(r"[1-9][0-9]{0,9}", value), "invalid ledger XID")
    low = int(value)
    require(3 <= low < XID_RANGE, "invalid normal XID")
    start, end = bounds
    full = start + (low - start) % XID_RANGE
    require(start < full < end, "ledger XID outside full XID bracket")
    return full


def parse_export(payload, pid, enabled):
    lines = payload.splitlines()
    require(payload.endswith("\n") and 2 <= len(lines) <= CAPACITY + 2, "truncated or oversized backend export")
    version, seen_pid, mode, exit_code, begin, finish = lines[0].split()
    require((version, int(seen_pid), int(mode), int(exit_code)) == ("postflush-v1", pid, enabled, 0),
            "backend identity, mode or exit status differs")
    begin, finish = int(begin), int(finish)
    require(0 < begin <= finish, "invalid backend clock bracket")
    tag, count, dropped, failures, asynchronous = lines[-1].split()
    require(tag == "end" and int(count) == len(lines) - 2, "incomplete backend export")
    require((int(dropped), int(failures), int(asynchronous)) == (0, 0, 0),
            "backend overflow, clock error or synchronous_commit changed")
    require(enabled or len(lines) == 2, "inactive backend emitted commit samples")
    records = {}
    last = begin
    for line in lines[1:-1]:
        xid, stamp = map(int, line.split())
        require(3 <= xid < 1 << 64 and xid % XID_RANGE >= 3 and xid not in records and last <= stamp <= finish,
                "invalid, repeated or unordered backend sample")
        records[xid] = stamp
        last = stamp
    return begin, finish, records


def join(ledger, runner_ids, logs, before, after, metadata, exports, enabled, count=1024):
    start, end = metadata["start"], metadata["end"]
    bounds = validate_clock(start, end)
    require(metadata["enabled"] is enabled and type(metadata["parallel_workers"]) is int
            and metadata["max_prepared_transactions"] == 0 and metadata["synchronous_standby_names"] == ""
            and metadata["wal_sync_method"] in {"fdatasync", "fsync", "fsync_writethrough", "open_datasync", "open_sync"},
            "unexpected hook mode or server settings")
    db = ledger["database"]
    require(db["version"] == "17.11" and all(db[key] is True for key in
            ("fsync", "full_page_writes", "synchronous_commit", "track_commit_timestamp", "tls")),
            "durability, commit timestamp or TLS settings differ")
    require(len(set(runner_ids)) == len(runner_ids) == 2, "unexpected runner roster")

    def app_roster(sessions):
        rows = [row for row in sessions if row["role"] == "compiled_caller"]
        pids = {row["backend"] for row in rows}
        require(len(pids) == len(rows) == 32 and all(row["tls"] is True for row in rows),
                "unexpected application session roster")
        return pids

    pids = app_roster(before)
    require(pids == app_roster(after) == exports.keys(), "missing, extra or replaced app backend")
    records = {}
    transactions = ledger["transactions"]
    require(len(transactions) == 1, "dense producer transaction count differs")
    producer = transactions[0]
    for pid, payload in exports.items():
        begin, finish, samples = parse_export(payload, pid, enabled)
        require(start["before_ns"] <= begin <= (producer["submitted_us"] + 1) * US
                and finish <= metadata["exports_collected_ns"], "backend started or exited outside arm bracket")
        for xid, stamp in samples.items():
            require(xid not in records and bounds[0] < xid < bounds[1], "global duplicate or out-of-window XID")
            records[xid] = (pid, stamp)

    jobs = {job["id"]: job for job in ledger["jobs"]}
    require(len(jobs) == len(ledger["jobs"]) == count, "dense ledger job count differs")
    seen = set()
    other = {lift_xid(producer["xid"], bounds)}
    rows = []
    for pid, log in zip(runner_ids, logs, strict=True):
        for line in log.splitlines():
            if not line.startswith("APP_POOL_JOB "):
                continue
            witness = json.loads(line.removeprefix("APP_POOL_JOB "))
            job_id = witness["job_id"]
            require(job_id in jobs and job_id not in seen, "missing or repeated handler identity")
            job = jobs[job_id]
            seen.add(job_id)
            require((job["worker_pid"], job["job_key"], job["entered_us"], job["attempt"])
                    == (pid, witness["job_key"], witness["entered_us"], witness["attempt"])
                    and job["attempt"] == job["effects"] == job["attempts"] == 1 and job["state"] == "completed",
                    "handler marker and ledger disagree")
            entered, lookup_start, lookup_end, first, second = (
                witness[key] for key in ("entered_us", "lookup_begin_us", "lookup_end_us",
                                         "first_return_us", "second_return_us"))
            require(start["before_ns"] <= entered * US and entered <= lookup_start <= lookup_end
                    <= first <= second <= job["settled_us"] and job["settled_us"] * US <= end["after_ns"],
                    "handler or settlement outside the arm clock")
            settled = lift_xid(job["settled_xid"], bounds)
            require(settled not in other, "producer or settlement XID reused")
            other.add(settled)
            for kind, left, right in (("attempt", lookup_end, first), ("effect", first, second)):
                xid = lift_xid(job[kind + "_xid"], bounds)
                record_us = job[kind + "_committed_us"]
                require(left <= record_us <= right, "commit record outside application call")
                row = dict(job_id=job_id, runner_pid=pid, kind=kind, xid=xid, call_begin_us=left,
                           record_us=record_us, return_us=right, record_to_return_us=right - record_us)
                if enabled:
                    require(xid in records, "application XID missing from hook")
                    backend, stamp = records.pop(xid)
                    require(record_us <= stamp // US <= right, "callback outside commit-record to return interval")
                    row.update(backend=backend, callback_ns=stamp, record_to_callback_us=stamp // US - record_us,
                               callback_to_return_us=right - stamp // US)
                rows.append(row)
    require(seen == jobs.keys() and not records, "unmatched handler or hook rows")
    app = {row["xid"] for row in rows}
    require(len(app) == 2 * count and not app & other, "application XID reused or confused with producer/settlement")
    return rows


def summarize(rows):
    phases = defaultdict(list)
    for row in rows:
        phases[row["kind"]].append(row)
        phases["both"].append(row)
    answer = {}
    for kind, group in phases.items():
        result = dict(n=len(group))
        for name in ("record_to_return_us", "record_to_callback_us", "callback_to_return_us"):
            if name not in group[0]:
                continue
            values = sorted(row[name] for row in group)
            result[name] = dict(total=sum(values), p50=median(values), p95=values[(95 * len(values) + 99) // 100 - 1])
        if "record_to_callback_us" in result:
            before, after = (result[name]["total"] for name in ("record_to_callback_us", "callback_to_return_us"))
            require(before + after > 0, "nonpositive cumulative record-to-return duration")
            result["record_to_callback_share"] = before / (before + after)
            result["conservative_share"] = [max(0, before - len(group)) / (before + after + len(group)),
                                            min(1, (before + len(group)) / max(1, before + after - len(group)))]
        answer[kind] = result
    return answer
