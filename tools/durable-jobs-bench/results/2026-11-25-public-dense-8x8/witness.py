"""Join successful terminal XIDs to owned runner reports in logger-off arms."""
from collections import Counter, defaultdict
import json

from verify import require, xid

SCALAR = "SELECT coalesce(mesh_jobs.settle_state($1, $2::uuid, $3::uuid, $4::jsonb, $5), $6)"
BULK = "SELECT ordinal::text FROM mesh_jobs.complete_batch_v1($1,$2::uuid[],$3::uuid[])"


def sql_total(statements, query, anchor):
    matches = [stmt for stmt in statements if anchor in stmt["query"]]
    require(len(matches) <= 1 and all(stmt["query"] == query for stmt in matches),
            "unexpected terminal SQL")
    if not matches:
        return 0, 0
    calls, rows = (matches[0][key] for key in ("calls", "rows"))
    require(type(calls) is type(rows) is int and calls > 0 and rows > 0, "invalid terminal SQL counters")
    return calls, rows


def logger_off(jobs, queue, logs, pids, namespace, grouped):
    require(len(logs) == len(pids) == len(set(pids)) == 8, "wrong logger-off roster")
    owned = defaultdict(Counter)
    waiting = defaultdict(Counter)
    owners = {}
    for job in jobs:
        pid, stop = job["worker_pid"], job["settled_us"]
        terminal = xid(job["settled_xid"], "terminal")
        require(pid in pids and owners.setdefault(terminal, (pid, stop)) == (pid, stop),
                "terminal transaction mixed runners or commit stamps")
        owned[pid][terminal] += 1
        waiting[pid][job["effect_committed_us"]] += 1
        waiting[pid][stop] -= 1

    widths = Counter()
    reports = {}
    for pid, log in zip(pids, logs, strict=True):
        lines = log.splitlines()
        require(len(lines) == 2 and lines[0] == "READY", "unexpected logger-on or missing runner report")
        report = json.loads(lines[1])
        sizes = Counter(owned[pid].values())
        require(report["namespace"] == namespace and report["claimed"] == report["finished"]
                == report["outcomes"]["completed"] == sum(owned[pid].values()) > 0,
                "terminal XIDs do not match the owned runner report")
        widths.update(sizes)
        current = peak = 0
        for _, delta in sorted(waiting[pid].items()):
            current += delta
            peak = max(peak, current)
        require(current == 0 and peak <= 8, "post-effect seat bound escaped")
        reports[str(pid)] = dict(xids=dict(owned[pid]), widths=dict(sizes), post_effect_peak=peak)

    statements = queue["queue_statements"]
    scalar_calls, scalar_rows = sql_total(statements, SCALAR, "SELECT coalesce(mesh_jobs.settle_state(")
    bulk_calls, bulk_rows = sql_total(statements, BULK, "FROM mesh_jobs.complete_batch_v1(")
    bulk_singletons = widths[1] - scalar_calls
    multi_xids = sum(count for width, count in widths.items() if width > 1)
    multi_rows = sum(width * count for width, count in widths.items() if width > 1)
    require(scalar_rows == scalar_calls and 0 <= bulk_singletons <= bulk_calls
            and bulk_calls == multi_xids + bulk_singletons
            and bulk_rows == multi_rows + bulk_singletons
            and scalar_rows + bulk_rows == len(jobs), "terminal SQL and committed XIDs disagree")
    require(bool(multi_xids) == bool(bulk_calls) == grouped
            and queue["completion_cohort_queries"] == bulk_calls
            and queue["terminal_xids"] == len(owners) == sum(widths.values())
            and queue["largest_terminal_cohort"] == max(widths) <= 8, "wrong terminal XID scope")
    return dict(runners=reports, widths=dict(widths), scalar_singletons=scalar_calls,
                bulk_singletons=bulk_singletons, bulk_calls=bulk_calls, bulk_rows=bulk_rows,
                post_effect_seat_us=sum(job["settled_us"] - job["effect_committed_us"] for job in jobs))
