"""Bound host CPU from the retained sparse stamps and canonical dense ledgers."""
import argparse
import json
from pathlib import Path
import tarfile

from diagnose import config_for_replay, history
from reduce import HOST_FIELDS, run, tick_interval

# The archived worker reads /proc/stat before each epoch stamp. Because the
# loop is serial, the read is also after the previous point's epoch stamp.
# No historical per-process or finer data should be inferred from these bounds.
MARGIN_US = 2000


def host_interval(points, edges, hz, cpus, jobs):
    start, end = edges
    left, right = [i for i, p in enumerate(points) if p["epoch_us"] <= start - MARGIN_US], [
        i for i, p in enumerate(points) if i and points[i - 1]["epoch_us"] >= end + MARGIN_US]
    run.require(left and right, "no historical host bounds")
    outer = (left[-1], right[0])
    inner = [i for i, p in enumerate(points) if i and points[i - 1]["epoch_us"] >= start + MARGIN_US
             and p["epoch_us"] <= end - MARGIN_US]
    samples = points[max(0, outer[0] - 1):outer[1] + 1]
    offsets = [point["epoch_us"] * 1000 - point["monotonic_ns"] for point in samples]
    run.require(max(offsets) - min(offsets) <= MARGIN_US * 1000,
                "archived wall and monotonic stamps diverged")
    scale = 1000 / jobs
    wall_s = (end - start) / 1e6
    capacity = wall_s * len(cpus)

    def cpu_interval(fields, endpoints):
        first, last = (points[i]["cpu"] for i in endpoints)
        return tick_interval([(first[cpu][col], last[cpu][col]) for cpu in cpus for col in fields], hz)

    result = {}
    for name, fields in HOST_FIELDS.items():
        lo = cpu_interval(fields, (inner[0], inner[-1]))[0] if len(inner) >= 2 else 0.
        hi = cpu_interval(fields, outer)[1]
        run.require(lo <= min(capacity, hi), "archived host bounds contradict window")
        result[name] = [lo * scale, min(capacity, hi) * scale]
    return {
        "host_ms_per_job": result,
        "capacity_ms_per_job": capacity * scale,
        "outer_edge_max_ms": [
            (start - points[outer[0] - 1]["epoch_us"]) / 1000 if outer[0] else None,
            (points[outer[1]]["epoch_us"] - end) / 1000,
        ],
        "inner_frames": len(inner),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archived_series", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    config = config_for_replay()
    archived = history(args.archived_series.resolve(), config)
    block, = sorted((args.archived_series / "blocks").glob("01-noop.*"))
    cpus = [f"cpu{cpu}" for cpu in config["cpus"]]
    for label, arm in archived["arms"].items():
        inventory = config["lanes"][label]
        raw = block / f"measured-{label}" / f"measured-{inventory['engine']}" / "raw.tar.gz"
        with tarfile.open(raw, "r:gz") as archive:
            with archive.extractfile("host-samples.jsonl") as stream:
                points = [json.loads(line) for line in stream]
        arm["historical_host"] = {
            name: host_interval(points, time_us, config["clock_ticks"], cpus, arm["score"]["completed"])
            for name, time_us in arm["phase_boundaries_us"].items()
        }
    run.write_json(args.output, archived)


if __name__ == "__main__":
    main()
