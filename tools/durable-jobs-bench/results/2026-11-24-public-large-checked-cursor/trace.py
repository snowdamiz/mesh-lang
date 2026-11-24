"""Account for checked fallback transactions using PostgreSQL Execute timestamps."""
from collections import defaultdict
from datetime import datetime
from decimal import Decimal
import re

from audit import statement

PREFIX = re.compile(r"^(\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\.\d+) UTC \[(\d+)] \d+ (\S*) (\S*) \| (.*)")
DURATION = re.compile(r"^LOG:\s+duration: ([\d.]+) ms\s+(?:execute [^:]+|statement):\s+(.*)", re.S)


def records(log):
    record = None
    for line in log.splitlines():
        if match := PREFIX.match(line):
            if record:
                yield record[:2] + ("\n".join(record[2]),)
            stamp, pid, user, db, content = match.groups()
            when = datetime.fromisoformat(stamp + "+00:00")
            record = (int(when.timestamp()) * 1_000_000 + when.microsecond, pid, [content]) if (
                user, db) == ("parity_subject", "mesh") else None
        elif record and line.startswith("\t"):
            record[2].append(line)
    if record:
        yield record[:2] + ("\n".join(record[2]),)


def union(intervals):
    out = []
    for lo, hi in sorted(intervals):
        if hi <= lo:
            continue
        if out and out[-1][1] >= lo:
            out[-1] = (out[-1][0], max(out[-1][1], hi))
        else:
            out.append((lo, hi))
    return out


def analyze(log, start, end, terminal):
    pids = defaultdict(list)
    for stamp, pid, body in records(log):
        match = DURATION.match(body)
        if not match:
            continue
        if label := statement(match[2].strip().rstrip(";")):
            pids[pid].append(dict(label=label, end_us=stamp,
                                  begin_us=stamp-int(Decimal(match[1])*1000)))
    sequences = []
    length = 7
    for pid, queries in pids.items():
        for i in range(len(queries)-length+1):
            seq = queries[i:i+length]
            if [q["label"] for q in seq] == [
                    "begin", "set", "commit", "probe_due", "begin", "cursor", "commit"]:
                begin, select, commit, probe, _, cursor, _ = seq
                sequences.append(dict(pid=pid, set_begin_us=begin["begin_us"],
                                      set_select_begin_us=select["begin_us"],
                                      set_select_end_us=select["end_us"], set_commit_end_us=commit["end_us"],
                                      probe_begin_us=probe["begin_us"], cursor_execute_end_us=cursor["end_us"]))
    def coverage(stop):
        intervals = union((max(start, r["set_begin_us"]), min(stop, r["set_commit_end_us"]))
                          for r in sequences)
        # Endpoints rounded by the PG millisecond log prefix. This is elapsed
        # coverage of observed fallbacks, including waits/overlap, without
        # attributing downstream retry schedules or reporting recoverable wall.
        return sum(hi-lo for lo, hi in intervals)/1000
    return dict(sequences=sequences, before_final_grant_set_coverage_ms=coverage(end),
                before_terminal_set_coverage_ms=coverage(terminal),
                cross_enqueue=sum(r["set_begin_us"] < start < r["set_commit_end_us"] for r in sequences))
