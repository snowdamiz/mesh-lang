"""Read the pinned CPU counters before the producer submitted any dense jobs."""
import json
import subprocess
import tarfile
from argparse import ArgumentParser
from hashlib import sha256
from io import BytesIO

from grant_capacity_frontier import EVIDENCE, SEAL


def measure(data, digest, river, expected_submit=None):
    assert sha256(data).hexdigest() == digest
    with tarfile.open(fileobj=BytesIO(data)) as archive:
        def read(name):
            return json.load(archive.extractfile(name))

        def lines(name):
            return [json.loads(line) for line in archive.extractfile(name)]

        assert read("status.json")["exit_code"] == 0
        ledger = read("adapter/ledger.json")
        transaction, = ledger["transactions"]
        submit = transaction["submitted_us"]
        assert expected_submit is None or submit == expected_submit
        assert len(ledger["jobs"]) == 1024
        frames = lines("host-samples.jsonl")
        if river:
            idle = [s for s in lines("adapter/river/measured/samples.jsonl") if s["idle"]]
            assert read("adapter/river/config.json")["idle_ms"] == 3000
            assert all(s["database_running_rows"] == s["terminal"] == 0 for s in idle)
            assert idle[-1]["offset_ms"] - idle[0]["offset_ms"] >= 2900
            idle_calls = None
        else:
            raw = read("adapter/raw.json")
            idle = raw["idle_samples"]
            assert raw["idle_seconds"] >= 2.9
            assert all(s["executing"] == s["terminal"] == 0 for s in idle)
            idle_calls = raw["raw_idle_sql_calls"]

    # /proc/stat is read before its frame stamp and after the preceding stamp.
    # Leave the last 250 ms for last-moment preparation and roster collection.
    selected = []
    for previous, current in zip(frames, frames[1:]):
        wall = current["epoch_us"] - previous["epoch_us"]
        mono = (current["monotonic_ns"] - previous["monotonic_ns"]) / 1000
        assert wall > 0 and abs(wall - mono) < 1000
        if previous["epoch_us"] >= submit - 2_000_000 and current["epoch_us"] <= submit - 250_000:
            selected.append((previous, current))
    assert len(selected) >= 3
    before, first = selected[0]
    penultimate, last = selected[-1]
    ticks = [sum(last["cpu"][k][i] - first["cpu"][k][i] for k in ("cpu0", "cpu1"))
             for i in range(8)]
    assert min(ticks) >= 0 and sum(ticks) > 0
    return dict(raw_sha256=digest, idle_connections=[s["connections"] for s in idle],
                idle_threads=[s["threads"] for s in idle], full_idle_sql_calls=idle_calls,
                cpu_ticks=ticks,
                busy_percent=100 * sum(ticks[i] for i in (0, 1, 2, 5, 6)) / sum(ticks),
                read_span_us_bounds=[penultimate["epoch_us"] - first["epoch_us"],
                                     last["epoch_us"] - before["epoch_us"]])


def replay():
    release = json.loads((EVIDENCE / "controller-release.json").read_bytes())
    assert release["request_sha256"] == SEAL and release["exit_status"] == 0
    groups = json.loads((EVIDENCE / "scored-before-release.json").read_bytes())["groups"]
    result = {}
    for rotation, group in groups.items():
        block = EVIDENCE / "blocks" / f"{int(rotation):02}-noop"
        reconciled = json.loads((block / "reconciled.json").read_bytes())
        result[rotation] = {}
        for lane in ("B", "A1", "A2", "reference", "river", "oban"):
            name = "measured-" + lane
            assert group[name]["raw_sha256"] == reconciled[name]["raw_sha256"]
            assert group[name]["completed"] == group[name]["ordered_independent_writes"] == 1024
            assert group[name]["affinity"] == [0, 1]
            path, = (block / name).glob("*/raw.tar.gz")
            result[rotation][lane] = measure(path.read_bytes(), group[name]["raw_sha256"],
                                            lane == "river", group[name]["first_submit_us"])
    return result


def two_runner_controls():
    base = "cb4f825c10ad9a7caf284cff7c59c3d3da17741f:tools/durable-jobs-bench/results/2026-11-25-direct4-hot-idle/evidence/native"

    def blob(name):
        return subprocess.check_output(["git", "show", f"{base}/{name}"])

    release = json.loads(blob("controller/pilot-release.json"))
    assert release["exit_status"] == 0 and release["groups"] == [1, 2]
    hashes = {path: digest for digest, path in
              (line.split(maxsplit=1) for line in blob("SHA256SUMS").decode().splitlines())}
    result = {}
    for n in release["groups"]:
        rows = {}
        for lane in ("service2-a", "service2-b", "service4", "direct2", "direct4",
                     "direct4-100-a", "direct4-100-b", "direct4-16"):
            path = f"blocks/{n:02}-noop/measured-{lane}/measured-mesh/raw.tar.gz"
            rows[lane] = measure(blob(path), hashes[path], False)
        result[n] = rows
    return result


if __name__ == "__main__":
    parser = ArgumentParser(description=__doc__)
    parser.add_argument("--two-runner", action="store_true", help="Read the released Direct4 spin and worker controls")
    args = parser.parse_args()
    print(json.dumps(two_runner_controls() if args.two_runner else replay(), indent=2))
