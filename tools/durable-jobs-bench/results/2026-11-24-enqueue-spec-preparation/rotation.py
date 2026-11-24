#!/usr/bin/env python3
"""Run the sealed component on Linode or replay its retained raw rows offline."""
import hashlib
import itertools
import json
import platform
from pathlib import Path
import statistics
import subprocess
import sys

ORDER = list(itertools.permutations(("stock-a", "candidate", "stock-b")))
DECISION_MS = {"short": 5.0, "large": 40.0}
REPEATS = {"short": 96, "large": 8}
STAGES = ("untagged_decode", "validation_and_policies", "to_value", "serialization", "sha256")
BOOT = "90ec1c24-08c8-4bc1-aa28-79b6db1bd13a"
HOST = ("Linux", "x86_64", "6.8.0-149-generic")


def digest(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def write(path, data):
    path.write_text(json.dumps(data, sort_keys=True, indent=2, allow_nan=False) + "\n")


def counters():
    line, = (line for line in Path("/proc/stat").read_text().splitlines() if line.startswith("cpu0 "))
    fields = list(map(int, line.split()[1:9]))
    return {"ticks": sum(fields), "steal": fields[7]}


def verify_frames(home, profile):
    capture = json.loads((home / f"{profile}-capture.json").read_text())
    folder = home / "frames" / profile
    for index, pin in enumerate(capture["frames"]):
        path = folder / str(index)
        if path.stat().st_size != pin["bytes"] or digest(path) != pin["sha256"]:
            raise ValueError(f"changed fixture frame {path}")
    if len(list(folder.iterdir())) != len(capture["frames"]):
        raise ValueError("unexpected fixture frame")


def run(home, dest):
    assert (platform.system(), platform.machine(), platform.release()) == HOST
    assert Path("/proc/sys/kernel/random/boot_id").read_text().strip() == BOOT
    dest.mkdir()
    pins = json.loads((home / "build-pins.json").read_text())
    write(dest / "build-pins.json", pins)
    for profile in DECISION_MS:
        verify_frames(home, profile)
        (dest / f"{profile}-semantic.json").write_bytes((home / "verify" / f"{profile}.json").read_bytes())
    identity = {"uname": list(platform.uname()), "boot": BOOT, "affinity": [0],
                "nproc_all": subprocess.check_output(["nproc", "--all"], text=True).strip(),
                "disk": subprocess.check_output(["findmnt", "-no", "MAJ:MIN", "--target", home], text=True).strip(),
                "cpuinfo": Path("/proc/cpuinfo").read_text().split("\n\n")[0],
                "uptime": subprocess.check_output(["uptime"], text=True).strip()}
    assert identity["disk"] == "8:0"
    write(dest / "platform.json", identity)
    for index, order in enumerate(ORDER, 1):
        profiles = list(DECISION_MS) if index % 2 else list(reversed(DECISION_MS))
        for profile in profiles:
            binary = home / "bin" / profile
            image = digest(binary)
            assert image == pins[profile]["component_sha256"]
            for lane in [*order, "stage"]:
                before = counters()
                result = subprocess.run(["taskset", "-c", "0", binary, lane.split("-")[0],
                                         home / "frames" / profile, str(REPEATS[profile])],
                                        capture_output=True, text=True, timeout=90, check=False)
                after = counters()
                write(dest / f"{profile}-{index}-{lane}.json", {
                    "rotation": index, "order": list(order), "profile": profile, "lane": lane,
                    "binary_sha256": image, "before": before, "after": after,
                    "exit": result.returncode, "stdout": result.stdout, "stderr": result.stderr,
                })
                result.check_returncode()
    write(dest / "summary.json", replay(dest))


def observed(data):
    cpu = data["cpu_ns"] / data["repeats"] / 1_000_000
    wall = data["wall_ns"] / data["repeats"] / 1_000_000
    return cpu, wall


def stats(values):
    return {"median": statistics.median(values), "min": min(values), "max": max(values)}


def replay(dest):
    identity = json.loads((dest / "platform.json").read_text())
    assert tuple(identity["uname"][i] for i in (0, 4, 2)) == HOST
    assert identity["boot"] == BOOT and identity["affinity"] == [0] and identity["disk"] == "8:0"
    pins = json.loads((dest / "build-pins.json").read_text())
    report = {"platform": identity, "profiles": {}}
    for profile, threshold in DECISION_MS.items():
        semantic = json.loads((dest / f"{profile}-semantic.json").read_text())
        assert digest(dest / f"{profile}-semantic.json") == pins[profile]["setup_verification_sha256"]
        results, stages, raw = [], [], {}
        for index, order in enumerate(ORDER, 1):
            group = {}
            for lane in [*order, "stage"]:
                path = dest / f"{profile}-{index}-{lane}.json"
                raw[path.name] = digest(path)
                row = json.loads(path.read_text())
                assert (row["profile"], row["rotation"], row["lane"], tuple(row["order"])) == (profile, index, lane, order)
                assert row["exit"] == 0 and not row["stderr"] and row["binary_sha256"] == pins[profile]["component_sha256"]
                elapsed_ticks = row["after"]["ticks"] - row["before"]["ticks"]
                steal_ticks = row["after"]["steal"] - row["before"]["steal"]
                assert elapsed_ticks > 0 and 0 <= steal_ticks / elapsed_ticks <= 0.01
                data = json.loads(row["stdout"])
                assert all(data[k] == semantic[k] for k in ("jobs", "frames", "output_sha256"))
                assert data["accepted"] == data["jobs"] * data["repeats"] and data["repeats"] == REPEATS[profile]
                assert data["cpu_ns"] > 0 and data["wall_ns"] > 0
                if lane == "stage":
                    assert all(t > 0 for t in data["stage_wall_ns"])
                    stages.append([t / data["repeats"] / 1_000_000 for t in data["stage_wall_ns"]])
                else:
                    assert data["stage_wall_ns"] == [0] * len(STAGES)
                    group[lane] = observed(data)
            results.append(group)
        cpu_savings = [min(r["stock-a"][0], r["stock-b"][0]) - r["candidate"][0] for r in results]
        wall_savings = [min(r["stock-a"][1], r["stock-b"][1]) - r["candidate"][1] for r in results]
        spread = [abs(r["stock-a"][0] - r["stock-b"][0]) for r in results]
        wins = sum(cpu > noise and wall > 0 for cpu, noise, wall in zip(cpu_savings, spread, wall_savings))
        report["profiles"][profile] = {
            "arm_cpu_ms": {k: stats([r[k][0] for r in results]) for k in ORDER[0]},
            "arm_wall_ms": {k: stats([r[k][1] for r in results]) for k in ORDER[0]},
            "stock_stage_wall_ms_diagnostic_only": {k: stats(list(v)) for k, v in zip(STAGES, zip(*stages))},
            "cpu_saving_vs_faster_stock_ms": cpu_savings, "wall_saving_vs_faster_stock_ms": wall_savings,
            "duplicate_cpu_spread_ms": spread, "wins_over_jitter": wins,
            "minimum_ms": threshold,
            "advance_to_broker_trial": statistics.median(cpu_savings) >= threshold and wins >= 5,
            "raw_sha256": raw,
        }
    return report


if __name__ == "__main__":
    command, *paths = sys.argv[1:]
    if command == "run":
        run(*(Path(p).resolve() for p in paths))
    elif command == "replay":
        print(json.dumps(replay(Path(paths[0])), sort_keys=True, indent=2, allow_nan=False))
    else:
        sys.exit("expected run or replay")
