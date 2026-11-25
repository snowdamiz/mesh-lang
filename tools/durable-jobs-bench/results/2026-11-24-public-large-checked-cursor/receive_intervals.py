"""Recover the Execute-to-COMMIT interval for native LARGE checked cursor grants."""
from collections import Counter
from datetime import datetime
from decimal import Decimal
from hashlib import sha256
import io
import json
from pathlib import Path
import re
import tarfile

from audit import statement
from trace import DURATION, union

HERE = Path(__file__).resolve().parent
SCORES = {
    "default": "3e8c6b4be2eca06025f78d464467916f8dd713788f0623ce39e9b53e85b439c6",
    "external-4m": "60b8d1021c001b64aeec4a07b5bbac5db1fcafd9097ee6f2b1e39d7ad936cddb",
}
PREFIX = re.compile(r"^(\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\.\d{3}) UTC \[(\d+)\] (\d+) parity_subject mesh \| ", re.M)


def receive_windows(log):
    headers = list(PREFIX.finditer(log))
    pending = {}
    result = []
    for index, head in enumerate(headers):
        stamp, pid, number = head.groups()
        stop = headers[index + 1].start() if index + 1 < len(headers) else len(log)
        body = log[head.end():stop].splitlines()
        # Other loggers may write before the next parity_subject message.
        sql_record = [body[0]]
        for line in body[1:]:
            if not line.startswith("\t"):
                break
            sql_record.append(line)
        event = DURATION.match("\n".join(sql_record))
        label = statement(event[2].strip().rstrip(";")) if event else None
        number = int(number)
        time = datetime.fromisoformat(stamp + "+00:00")
        end = int(time.timestamp()) * 1_000_000 + time.microsecond
        if pid in pending:
            prior, begin, select_end = pending.pop(pid)
            assert label == "commit" and number == prior + 1, (pid, prior, number, label)
            duration = int(Decimal(event[1]) * 1000)
            result.append((begin, select_end, end - duration, end))
        if label == "cursor":
            assert "execute <unnamed>:" in sql_record[0]
            duration = int(Decimal(event[1]) * 1000)
            pending[pid] = (number, end - duration, end)
    assert not pending
    return result


def bounds(windows):
    # Millisecond log prefixes are truncated. The duration has microsecond resolution.
    lower = union((end + 1000, commit - 1) for _, end, commit, _ in windows)
    upper = union((end, commit + 1001) for _, end, commit, _ in windows)
    return [sum(hi - lo for lo, hi in intervals) for intervals in (lower, upper)]


def analyze(raw, score):
    data = raw.read_bytes()
    assert sha256(data).hexdigest() == score["sha256"]
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        log = archive.extractfile("adapter/postgres.log").read().decode()
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
    assert ledger["version"] == "b3557273d72be3802bf189b284e7caaf5f3caa0f"
    txn, = ledger["transactions"]
    jobs = ledger["jobs"]
    assert len(jobs) == len({j["id"] for j in jobs}) == score["cursor_grant_rows"] == 64
    assert not score["set_grant_rows"]
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid", "settled_xid")}) == 192
    windows = receive_windows(log)
    assert len(windows) == score["sql"]["cursor"]["calls"]
    positives = len(windows) - score["cursor_contention_hints"]
    assert score["sql"]["cursor"]["rows"] == 64 + score["cursor_contention_hints"]
    candidates = []
    for job in jobs:
        # Each floored claim stamp falls inside its successful cursor Execute;
        # the handler entry follows that transaction's COMMIT reply.
        stamp = job["claimed_us"]
        assert txn["committed_us"] <= stamp <= job["entered_us"] <= job["attempt_committed_us"]
        possible = [i for i, (begin, end, _, commit_end) in enumerate(windows)
                    if begin - 1 <= stamp + 999 and stamp < end + 1000
                    and commit_end <= job["entered_us"]]
        assert possible
        candidates.append(possible)
    # If the uniquely located jobs account for all productive cursor calls,
    # every remaining job must belong to one of those same calls.
    productive = {opts[0] for opts in candidates if len(opts) == 1}
    assert len(productive) == positives
    hits = Counter()
    owners = {}
    stamps = {}
    for job, opts in zip(jobs, candidates):
        i, = productive.intersection(opts)
        hits[i] += 1
        stamps.setdefault(i, []).append(job["claimed_us"])
        assert owners.setdefault(i, job["worker_pid"]) == job["worker_pid"]
    assert max(hits.values()) <= 8
    ordered = sorted(hits, key=lambda i: windows[i][1])
    # Stock stamps a claim after rendering the row and before its UPDATE.
    # Keep timestamp precision when bounding the elapsed union on the host.
    def after_render(i):
        begin, _, commit, end = windows[i]
        return begin, max(stamps[i]), commit, end

    rendered = [after_render(i) for i in ordered]
    after_render_before_execute_end = [
        (0, max(stamps[i]), windows[i][1], 0) for i in ordered
    ]
    # A hypothetical pipeline which starts delivery at the first row stamp
    # can only move the tail into the observed first-to-last stamp window.
    # Keep other work fixed and even credit the still-required last row.
    overlap_ceiling = sum(min(max(stamps[i]) - min(stamps[i]) + 999,
                              max(0, windows[i][2] + 1001 - max(stamps[i])))
                          for i in ordered if hits[i] > 1)
    return dict(raw_sha256=score["sha256"], cursor_calls=len(windows),
                contention_hints=score["cursor_contention_hints"],
                grant_sizes=[hits[i] for i in ordered],
                fixed_stamp_pipeline_credit_ceiling_us=overlap_ceiling,
                positive_post_render_to_execute_end_union_us=bounds(after_render_before_execute_end),
                last_grant_post_render_to_commit_start_us=bounds(rendered[-1:]),
                positive_post_render_to_commit_start_union_us=bounds(rendered),
                last_grant_post_execute_us=bounds([windows[ordered[-1]]]),
                positive_post_execute_union_us=bounds([windows[i] for i in ordered]))


def replay():
    result = {}
    for storage, digest in SCORES.items():
        root = HERE / "evidence/native" / storage
        data = (root / "summary.json").read_bytes()
        assert sha256(data).hexdigest() == digest
        report = json.loads(data)
        assert not report["gate"]["advance"] and not report["failed"]
        result[storage] = []
        for _, (folder, group) in sorted(report["groups"].items()):
            score = group["arms"]["measured-trace"]
            raw = root / "blocks" / folder / "measured-trace/measured-mesh/raw.tar.gz"
            result[storage].append(analyze(raw, score))
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2))
