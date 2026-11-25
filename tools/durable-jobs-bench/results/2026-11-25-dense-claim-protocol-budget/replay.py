"""Price the observed BEGIN boundary and unnamed statement phases in native logs."""
from collections import Counter, defaultdict
from datetime import datetime, timezone
from hashlib import sha256
import json
from pathlib import Path
import re
import tarfile

HERE = Path(__file__).resolve().parent
CAPTURE = HERE.parent / "2026-11-24-public-dense-set-claim/evidence/native"
HEADER = re.compile(
    r"^(\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\.\d{3}) UTC \[(\d+)\] (\d+) parity_subject mesh "
    r"\| LOG:  duration: (\d+)\.(\d{3}) ms  (statement|parse <unnamed>|bind <unnamed>|execute <unnamed>): (.*)$"
)


def kind(sql):
    if sql == "BEGIN ISOLATION LEVEL READ COMMITTED":
        return "begin"
    if sql == "COMMIT":
        return "commit"
    if sql.startswith("SELECT status, id, token, context, timeout_ms, context_bytes,"):
        return "grant"
    if sql.startswith("SELECT coalesce(mesh_jobs.settle_state("):
        return "settle"
    return "other"


def union_us(intervals):
    total = 0
    previous_end = 0
    for start, stop in sorted(intervals):
        total += max(0, stop - max(start, previous_end))
        previous_end = max(previous_end, stop)
    return total


def summarize(intervals):
    lower = [(start + 1000, stop - 1) for start, stop in intervals if stop - start > 1001]
    upper = [(start, stop + 1001) for start, stop in intervals if stop + 1001 > start]
    return dict(adjacent_pairs=len(intervals), strictly_positive_lower=len(lower),
                sum_lower_us=sum(b-a for a,b in lower), sum_upper_us=sum(b-a for a,b in upper),
                union_lower_us=union_us(lower), union_upper_us=union_us(upper))


def arm(raw, score):
    assert sha256(raw.read_bytes()).hexdigest() == score["raw_sha256"] and score["completed"] == 1024
    first, last = score["first_commit_us"], score["last_settlement_us"]
    sums, counts = defaultdict(Counter), defaultdict(Counter)
    before_select, after_select, previous = [], [], {}
    with tarfile.open(raw) as archive:
        for line in archive.extractfile("adapter/postgres.log"):
            m = HEADER.match(line.decode().rstrip("\n"))
            if not m:
                continue
            stamp, pid, number, integer, frac, event, sql = m.groups()
            dt = datetime.strptime(stamp, "%Y-%m-%d %H:%M:%S.%f").replace(tzinfo=timezone.utc)
            end = int(dt.timestamp()) * 1_000_000 + dt.microsecond
            duration = int(integer) * 1000 + int(frac)
            op, phase = kind(sql), event.split()[0]
            # Bracket the logged millisecond and three-decimal duration. Count
            # only records whose whole reported interval is inside the drain.
            inside = first <= end-duration-1 and end+1000 <= last
            if inside and op != "other":
                sums[op][phase] += duration
                counts[op][phase] += 1
            prior = previous.get(pid)
            if prior is not None and inside:
                number0, end0, op0, phase0, prior_inside = prior
                if prior_inside and int(number) == number0 + 1:
                    if (op0, phase0, op, phase) == ("begin", "statement", "grant", "parse"):
                        before_select.append((end0, end-duration))
                    if (op0, phase0, op, phase) == ("grant", "execute", "commit", "statement"):
                        after_select.append((end0, end-duration))
            previous[pid] = (int(number), end, op, phase, inside)
    assert counts["settle"]["execute"] >= 1015
    assert counts["grant"]["parse"] - 2 <= len(before_select) <= counts["grant"]["parse"]
    assert counts["grant"]["execute"] - 2 <= len(after_select) <= counts["grant"]["execute"]
    assert min(len(before_select), len(after_select)) >= 50
    return dict(raw_sha256=score["raw_sha256"], phases_us=sums, counts=counts,
                begin_to_parse=summarize(before_select), execute_to_commit=summarize(after_select))



def replay():
    src = json.loads((CAPTURE / "final-replay.json").read_bytes())
    assert not src["failed"]
    result = {}
    for group, (name, block) in sorted(src["groups"].items()):
        for label in ("equal-trace", "stock-trace"):
            score = block["arms"][label]
            path = CAPTURE / "blocks" / name / ("measured-" + label) / "measured-mesh/raw.tar.gz"
            result[f"{group}/{label}"] = arm(path, score)
    assert len(result) == 4
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), sort_keys=True, indent=2))
