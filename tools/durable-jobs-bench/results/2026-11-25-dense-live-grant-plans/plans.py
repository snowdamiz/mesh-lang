"""Replay PostgreSQL 17 auto_explain objects and exact statement IDs."""
from collections import Counter
import json
import re

HEADER = re.compile(r"^dense-plan\|(\d+)\.(\d{3})\|(\d+)\|(\d+)\|([^|]*)\|([^|]*)\| (.*)$")
DURATION = re.compile(r"^LOG:\s+duration: ([\d.]+) ms\s+plan:\s*(.*)", re.S)


def records(log):
    current = None
    for line in log.splitlines():
        if match := HEADER.match(line):
            if current:
                yield current
            sec, ms, pid, num, role, db, first = match.groups()
            current = (int(sec) * 1_000_000 + int(ms) * 1000, int(pid), int(num), role, db, [first])
        elif current and line.startswith("\t"):
            current[-1].append(line)
        elif current:
            yield current
            current = None
    if current:
        yield current


def extract(log):
    for end, pid, line, role, db, lines in records(log):
        if match := DURATION.match("\n".join(lines)):
            duration, body = match.groups()
            data, _ = json.JSONDecoder().raw_decode(body)
            if not isinstance(data, dict) or "Plan" not in data or "Query Text" not in data:
                raise ValueError(f"malformed auto_explain object at PID {pid} log line {line}")
            yield dict(end_us=end, pid=pid, log_line=line, session_role=role, database=db,
                       duration_ms=float(duration), query_id=data.get("Query Identifier"),
                       query=data["Query Text"], plan=data["Plan"])


SQL_LITERAL = re.compile(r"'(?:[^']|'')*'")
LITERAL_ALLOW = {"'available'", "'scheduled'", "'retryable'", "'executing'", "'completed'",
                 "'active'", "'paused'", "'client_encoding'"}


def safe(value):
    # Keep predicate structure and the synthetic state labels. Replace literal
    # arguments and use a stable placeholder for lease-marker identifiers.
    if isinstance(value, str):
        value = SQL_LITERAL.sub(lambda m: m[0] if m[0] in LITERAL_ALLOW else "'<literal>'", value)
        return re.sub("token", "lease_marker", value, flags=re.I)
    if isinstance(value, list):
        return [safe(item) for item in value]
    if isinstance(value, dict):
        return {key: safe(item) for key, item in value.items()}
    return value


def classify(query):
    q = " ".join(query.lower().split())
    if "from mesh_jobs.claim_batch_set_v1(" in q:
        return "claim"
    if q.startswith("with picked as materialized") and "update mesh_jobs.jobs" in q:
        return "grant"
    if q.startswith("with held as materialized") and "update mesh_jobs.jobs" in q:
        return "group_update"
    if "mesh_jobs.complete_batch_v1(" in q:
        return "group_settle"
    if "mesh_jobs.settle_state(" in q:
        return "scalar_settle"
    if q.startswith("update mesh_jobs.jobs set state = next_state"):
        return "scalar_update"
    if "from mesh_jobs.jobs" in q and "id = job_id and lease_lease_marker = lease_marker" in q and "nowait" in q:
        return "scalar_lock"
    if "mesh_jobs.recover(" in q:
        return "recover"
    if q.startswith("with due as") and "update mesh_jobs.jobs" in q:
        return "recover_due"
    if "from mesh_jobs.jobs" in q and "order by lease_expires_at_ms, id" in q:
        return "recover_expired"
    if q.startswith("remaining :=") and "count(*) from mesh_jobs.jobs" in q:
        return "capacity"
    if "from mesh_jobs.queues" in q and "for no key update" in q:
        return "queue_lock"
    if "exists" in q and "from mesh_jobs.jobs" in q and "mesh_jobs.batch_context_blocks" in q:
        return "eligible"
    if "mesh_jobs.jobs" in q:
        return "other_jobs"
    return None


def nodes(plan, path=()):
    node = {key: value for key, value in plan.items() if key != "Plans"}
    if "Actual Rows" not in node or "Actual Loops" not in node:
        raise ValueError("EXPLAIN lacks actual rows/loops")
    rows, loops = node["Actual Rows"], node["Actual Loops"]
    yield dict(node, subtree_path=list(path), emitted_approx=rows*loops,
               removed_approx={key: value*loops for key, value in node.items()
                               if key.startswith("Rows Removed by")})
    for i, child in enumerate(plan.get("Plans", [])):
        yield from nodes(child, (*path, i))


def reduce(plans, frames, closed, clocks):
    initial, primed, final = frames
    keys = ("userid", "dbid", "toplevel", "queryid")
    counters = ("calls", "rows", "total_exec_time", "shared_blks_hit", "shared_blks_read",
                "shared_blks_dirtied", "shared_blks_written", "temp_blks_read", "temp_blks_written",
                "wal_records", "wal_fpi", "wal_bytes")

    def indexed(frame):
        result = {tuple(row[k] for k in keys): row for row in frame["statements"]}
        if len(result) != len(frame["statements"]):
            raise ValueError("duplicate statement key")
        return result

    begin, end, flushed = map(indexed, (primed, final, closed))
    deltas = []
    for key, row in flushed.items():
        if not row["query"] or row["query"] == "[redacted]":
            raise ValueError("statement text missing in flushed snapshot")
        old, observed = begin.get(key, {}), end.get(key, {})
        delta = {k: row[k] - old.get(k, 0) for k in counters}
        if min(delta.values()) < 0 or observed.get("calls", 0) > row["calls"]:
            raise ValueError("statement counters regressed")
        deltas.append(dict(**{k: row[k] for k in (*keys, "role", "query")},
                           category=classify(row["query"]), closed_delta=delta,
                           observed_delta={k: observed.get(k, 0)-old.get(k, 0) for k in counters}))

    participants = {}
    sessions = {s["client_port"]: s for s in initial["sessions"]}
    for runner, ports in initial["runner_ports"].items():
        matched = [sessions[port] for port in ports if port in sessions]
        if not any(row["usename"] == "parity_subject" and row["ssl"] for row in matched):
            raise ValueError(f"runner {runner} lacks a matched TLS participant backend")
        participants[runner] = [row["pid"] for row in matched
                                if row["usename"] == "parity_subject" and row["ssl"]]

    samples = []
    counts = Counter()
    for record in plans:
        if (record["session_role"], record["database"]) != ("parity_subject", "mesh"):
            raise ValueError("auto_explain leaked outside the participant")
        category = classify(record["query"])
        if not category:
            continue
        finish = record["end_us"]
        start = finish - round(record["duration_ms"]*1000)
        window = "outside"
        for name, lo, hi in (
            ("idle", initial["begin_us"], primed["end_us"]),
            ("submit", clocks["first_submit_us"], clocks["first_commit_us"]),
            ("drain", clocks["first_commit_us"], clocks["last_settlement_us"]),
        ):
            if lo + 1000 <= start and finish + 1000 <= hi:
                window = name
                break
            if start < hi and finish >= lo:
                window = "boundary"
        mapping = [r for r in deltas if r["queryid"] == record["query_id"] and r["category"] == category]
        if window in ("submit", "drain") and len(mapping) != 1:
            raise ValueError(f"live {category} id {record['query_id']} has {len(mapping)} statement keys")
        if len(mapping) == 1 and window in ("submit", "drain") and mapping[0]["closed_delta"]["calls"] <= 0:
            raise ValueError(f"live {category} lacks a positive call delta")
        counts[(category, window)] += 1
        samples.append(dict(record, category=category, window=window,
                            statement_keys=[{k: r[k] for k in (*keys, "role")} for r in mapping],
                            nodes=list(nodes(record["plan"]))))

    relations = [{k: frame["relation"][k] for k in ("seq_scan", "seq_tup_read", "idx_scan", "idx_tup_fetch",
                                                   "n_tup_ins", "n_tup_upd", "n_dead_tup",
                                                   "n_tup_hot_upd", "n_tup_newpage_upd")}
                 for frame in frames + [closed]]
    if len({(frame["db"], frame["relation"]["relid"], frame["dealloc"], frame["statements_reset"],
             frame["version_num"]) for frame in frames + [closed]}) != 1:
        raise ValueError("counter identity drift, eviction or reset")
    index_by_phase = [{i["indexrelid"]: i for i in frame["indexes"]} for frame in (primed, final, closed)]
    if any(set(data) != set(index_by_phase[0]) for data in index_by_phase[1:]):
        raise ValueError("index roster drift")
    indexes = {}
    for oid, row in index_by_phase[0].items():
        indexes[row["indexrelname"]] = {
            name: {k: later[oid][k] - row[k] for k in ("idx_scan", "idx_tup_read", "idx_tup_fetch")}
            for name, later in zip(("observed", "closed"), index_by_phase[1:], strict=True)
        }
    return dict(samples=samples, sample_counts={f"{a}/{b}": n for (a,b),n in counts.items()},
                participants=participants, statement_deltas=deltas, relations=relations, indexes=indexes)
