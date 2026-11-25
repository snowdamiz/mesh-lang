"""Join physical-host sample intervals to confirmed Mesh effects and queue lock timeouts."""
import argparse
from bisect import bisect_right
from datetime import datetime, timezone
from hashlib import sha256
import io
import json
from pathlib import Path
import re
import tarfile

from late_capacity_peer_phases import SEAL

LOCK_SQL = "SELECT FROM mesh_jobs.queues WHERE namespace = $1 AND name = $2 FOR NO KEY UPDATE"
LOCK_ERROR = re.compile(r"^([\d-]+ [\d:.]+) UTC \[(\d+)\] ERROR:  canceling statement due to lock timeout$")
EPOCH = datetime(1970, 1, 1, tzinfo=timezone.utc)


def queue_timeout_records(lines):
    stamps = []
    for i, line in enumerate(lines):
        error = LOCK_ERROR.fullmatch(line)
        if not error:
            continue
        stamp, pid = error.groups()
        # Interleaved PostgreSQL processes can log between ERROR and STATEMENT.
        prefix = f"{stamp} UTC [{pid}] STATEMENT:  "
        index = next(j for j in range(i + 1, len(lines)) if lines[j].startswith(prefix))
        statement = lines[index][len(prefix):]
        context = " ".join(" ".join(lines[i + 1:index]).split())
        if statement == LOCK_SQL:
            site = "prelock"
        elif (f'{stamp} UTC [{pid}] CONTEXT: SQL statement "SELECT * FROM mesh_jobs.queues '
              'WHERE namespace = ns AND name = queue_name FOR NO KEY UPDATE"' in context
              and "PL/pgSQL function mesh_jobs.claim_batch_bounded_v1(" in context):
            site = "bounded_claim"
        else:
            raise ValueError(f"Unclassified lock timeout in PID {pid} at {stamp}")
        instant = datetime.fromisoformat(stamp).replace(tzinfo=timezone.utc)
        stamps.append(((instant - EPOCH) // datetime.resolution, site, int(pid)))
    return stamps


def queue_timeouts(lines):
    return [(stamp, site) for stamp, site, _ in queue_timeout_records(lines)]


def device_busy_ms(sample):
    fields, = [row.split()[3:] for row in sample["diskstats"] if row.split()[:2] == ["8", "0"]]
    return int(fields[9])


def measure(raw, score):
    data = raw.read_bytes()
    assert sha256(data).hexdigest() == score["raw_sha256"]
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
        samples = [json.loads(line) for line in archive.extractfile("host-samples.jsonl")]
        timeouts = queue_timeouts(archive.extractfile("adapter/postgres.log").read().decode().splitlines())
    effects = sorted(job["effect_committed_us"] for job in ledger["jobs"])
    origin, = ledger["transactions"]
    start = origin["committed_us"]
    end = max(job["settled_us"] for job in ledger["jobs"])
    assert len(effects) == 1024 and end - start == score["drain_us"]
    intervals = []
    for a, b in zip(samples, samples[1:]):
        assert a["epoch_us"] < b["epoch_us"] and a["monotonic_ns"] < b["monotonic_ns"]
    # The sealed collector reads CPU before stamping the frame, and disk after.
    # Use neighboring stamps to enclose both physical reads at each endpoint.
    for previous, left, right, following in zip(samples, samples[1:], samples[2:], samples[3:]):
        t0, t1 = left["epoch_us"], right["epoch_us"]
        if previous["epoch_us"] < start or following["epoch_us"] > end:
            continue
        ticks = [sum(right["cpu"][cpu][i] - left["cpu"][cpu][i] for cpu in ("cpu0", "cpu1"))
                 for i in range(8)]
        assert min(ticks) >= 0
        # The log truncates to ms. Refuse to assign an error to either side of an uncertain edge.
        assert all(not (stamp <= edge < stamp + 999) for stamp, _ in timeouts for edge in (t0, t1))
        intervals.append(dict(start_from_commit_us=t0 - start, duration_us=t1 - t0,
                              earliest_cpu_read_from_commit_us=previous["epoch_us"] - start,
                              latest_disk_read_from_commit_us=following["epoch_us"] - start,
                              effects=bisect_right(effects, t1) - bisect_right(effects, t0),
                              disk_busy_ms=device_busy_ms(right) - device_busy_ms(left),
                              iowait_ticks=ticks[4], total_ticks=sum(ticks),
                              queue_timeouts=sum(t0 < stamp <= t1 for stamp, _ in timeouts)))
    disk_elapsed_us = None
    if intervals:
        first, last = intervals[0], intervals[-1]
        first_stamp = first["start_from_commit_us"]
        last_stamp = last["start_from_commit_us"] + last["duration_us"]
        # Disk reads follow their own frame stamp and precede the next stamp.
        disk_elapsed_us = [last_stamp - first_stamp - first["duration_us"],
                           last["latest_disk_read_from_commit_us"] - first_stamp]
    return dict(drain_us=end - start, raw_sha256=score["raw_sha256"], intervals=intervals,
                disk_elapsed_us_bounds=disk_elapsed_us,
                drain_queue_timeouts={site: sum(start < stamp and stamp + 999 <= end and why == site
                                               for stamp, why in timeouts)
                                      for site in ("prelock", "bounded_claim")})


def replay(evidence):
    suite = Path(__file__).resolve().parents[2]
    digest = sha256((suite / "worker.py").read_bytes()).hexdigest()
    assert digest == "16d1e01d19ee7b325908f07a69c6cdc772caf7d034ad79a6583494d2e107993b"
    for lane in ("A1", "B"):
        inventory = json.loads((evidence.parent / "staging" / lane / "inventory.json").read_bytes())
        files = inventory["engines"]["mesh"]["verified_files"]
        assert [value for path, value in files.items() if path.endswith("/worker.py")] == [digest]
    release = json.loads((evidence / "controller-release.json").read_bytes())
    assert release["request_sha256"] == SEAL and release["exit_status"] == 0
    scores = json.loads((evidence / "scored-before-release.json").read_bytes())["groups"]
    groups = {}
    for number in range(1, 6):
        block = evidence / "blocks" / f"{number:02}-noop"
        groups[number] = {}
        for lane in ("A1", "B", "A2"):
            cell = "measured-" + lane
            groups[number][lane] = measure(block / cell / "measured-mesh/raw.tar.gz", scores[str(number)][cell])
    return groups


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path)
    print(json.dumps(replay(parser.parse_args().evidence), indent=2, sort_keys=True))
