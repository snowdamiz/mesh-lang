"""Read the pinned CPU counters before the producer submitted any dense jobs."""
import json
import tarfile
from hashlib import sha256

from grant_capacity_frontier import EVIDENCE, SEAL


def measure(path, score, river):
    digest = sha256(path.read_bytes()).hexdigest()
    assert digest == score["raw_sha256"]
    assert score["completed"] == score["ordered_independent_writes"] == 1024
    assert score["affinity"] == [0, 1]
    with tarfile.open(path) as archive:
        def read(name):
            return json.load(archive.extractfile(name))

        def lines(name):
            return [json.loads(line) for line in archive.extractfile(name)]

        assert read("status.json")["exit_code"] == 0
        transaction, = read("adapter/ledger.json")["transactions"]
        submit = transaction["submitted_us"]
        assert submit == score["first_submit_us"]
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
            path, = (block / name).glob("*/raw.tar.gz")
            result[rotation][lane] = measure(path, group[name], lane == "river")
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2))
