"""Reduce PostgreSQL diagnostic logs into separately scoped claim observations."""

from collections import Counter
from datetime import datetime
import json
import re

PREFIX = re.compile(r"^(\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\.\d+) UTC \[(\d+)\] \d+ parity_subject mesh \| (.*)")
DURATION = re.compile(r"^LOG:\s+duration: ([\d.]+) ms\s+(execute [^:]+|statement):\s+(.*)", re.S)
PLAN = re.compile(r"^LOG:\s+duration: ([\d.]+) ms\s+plan:\s*(.*)", re.S)
LOCK = re.compile(r"^LOG:\s+process \d+ acquired .+ after ([\d.]+) ms", re.M)


def kind(query):
    q = " ".join(query.lower().split())
    if "from mesh_jobs.claim_batch_set_v1(" in q:
        return "outer"
    if q.startswith("with picked as materialized"):
        return "grant"
    if "from mesh_jobs.queues" in q and "for no key update" in q:
        return "queue_lock"
    if "count(" in q and "mesh_jobs.jobs" in q and "executing" in q:
        return "capacity_count"
    if "mesh_jobs.batch_context_blocks" in q and "mesh_jobs.jobs" in q:
        return "empty_eligibility"
    return None


def records(log):
    current = None
    for line in log.splitlines():
        match = PREFIX.match(line)
        if match:
            if current:
                yield current[0], current[1], "\n".join(current[2])
            stamp, pid, body = match.groups()
            micros = int(datetime.fromisoformat(stamp + "+00:00").timestamp() * 1_000_000 + 0.5)
            current = micros, pid, [body]
        elif line.startswith("\t") and current:
            current[2].append(line)
        else:
            if current:
                yield current[0], current[1], "\n".join(current[2])
            current = None
    if current:
        yield current[0], current[1], "\n".join(current[2])


def nodes(plan, parent=()):
    loc = (*parent, plan["Node Type"])
    rows, loops = plan.get("Actual Rows", 0), plan.get("Actual Loops", 0)
    yield dict(path=list(loc), relation=plan.get("Relation Name"), index=plan.get("Index Name"),
               actual_rows=rows, actual_loops=loops, emitted=rows*loops,
               removed={k: v*loops for k, v in plan.items() if k.startswith("Rows Removed by")},
               heap_fetches=plan.get("Heap Fetches"), sort_method=plan.get("Sort Method"))
    for node in plan.get("Plans", []):
        yield from nodes(node, loc)


def analyze(log, idle, submit, enqueue, terminal):
    plans, intervals, waits = [], [], []
    pending_waits, open_claims, latest_queue = {}, {}, {}
    bad_plan = 0
    timeout_count = 0
    other_waits = 0

    def window(begin, end):
        # PostgreSQL's log prefix is millisecond-resolution.
        for name, lo, hi in (("idle", idle[0], idle[1]), ("submit_to_enqueue", submit, enqueue),
                             ("committed_drain", enqueue, terminal)):
            if lo+1000 <= begin and end+1000 <= hi:
                return name
            if begin < hi+1000 and end > lo-1000:
                return "boundary"
        return "outside"

    for stamp, pid, body in records(log):
        if match := LOCK.match(body):
            # The DETAIL/CONTEXT line is a separate prefixed record. Attribute
            # acquired waits only to the matching next claim execution on this PID.
            pending_waits.setdefault(pid, []).append((stamp, float(match[1])))
        if "ERROR:" in body and "due to lock timeout" in body:
            timeout_count += 1
            pending_waits.pop(pid, None)
        if match := PLAN.match(body):
            try:
                top = json.JSONDecoder().raw_decode(match[2].lstrip())[0]
                if not isinstance(top, dict):
                    raise ValueError("auto_explain 17 emits a plan object")
            except (ValueError, TypeError):
                bad_plan += 1
                continue
            if category := kind(top.get("Query Text", "")):
                plan = top["Plan"]
                plans.append(dict(pid=pid, end_us=stamp, kind=category, elapsed_ms=float(match[1]),
                                  window=window(stamp-int(float(match[1])*1000), stamp),
                                  nodes=list(nodes(plan)), triggers=top.get("Triggers", []),
                                  buffers={k: v for k, v in plan.items() if "Blocks" in k}))
                if category == "queue_lock":
                    latest_queue[pid] = stamp
        if match := DURATION.match(body):
            ms, _, query = match.groups()
            ms = float(ms)
            begin = stamp-int(ms*1000)
            q = query.strip().upper().rstrip(";")
            observed = pending_waits.pop(pid, [])
            if kind(query) == "outer":
                wait = sum(t for when, t in observed if begin-1000 <= when <= stamp+1000)
                queue = latest_queue.pop(pid, None)
                if queue is not None and not begin-1000 <= queue <= stamp+1000:
                    queue = None
                claim = dict(pid=pid, begin_us=begin, end_us=stamp, execution_ms=ms,
                             acquired_lock_wait_lower_ms=min(ms, wait),
                             window=window(begin, stamp), queue_lock_statement_end_us=queue,
                             commit_end_us=None, finish_kind=None)
                intervals.append(claim)
                open_claims[pid] = claim
                if wait:
                    waits.append(dict(pid=pid, end_us=stamp, elapsed_ms=wait))
            else:
                other_waits += len(observed)
                if q in ("COMMIT", "ROLLBACK") and pid in open_claims:
                    claim = open_claims.pop(pid)
                    claim.update(commit_end_us=stamp, finish_kind=q, finish_execution_ms=ms,
                                 select_end_to_finish_start_ms=(begin-claim["end_us"])/1000)
    counts = Counter((x["window"], x["kind"]) for x in plans)
    return dict(claims=intervals, plan_counts={f"{a}/{b}": n for (a,b),n in counts.items()},
                plans=plans, incomplete_plan_records=bad_plan, acquired_waits=waits,
                observed_worker_role_lock_timeouts=timeout_count, waits_other_or_unclassified=other_waits)
