"""Fresh public-source entrant, with the original six-order LARGE decision."""
import json
from pathlib import Path
import platform
import statistics
import subprocess
import sys

import original_rotation as original

ORDER = original.ORDER
LANES = {"stock-a": "stock", "stock-b": "stock", "candidate": "candidate", "move": "move"}
PROFILES = ("plain", "escaped")
REPEATS = original.REPEATS["large"]
THRESHOLD = original.DECISION_MS["large"]


def load(path):
    return json.loads(path.read_text())


def run(home, out):
    assert (platform.system(), platform.machine(), platform.release()) == original.HOST
    boot = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
    assert boot == original.BOOT
    out.mkdir()
    for name in ("build-pins.json", "semantic.json"):
        (out / name).write_bytes((home / "verify" / name).read_bytes())
    pins = load(out / "build-pins.json")
    for lane in set(LANES.values()):
        assert original.digest(home / "bin" / lane) == pins[lane]["binary_sha256"]
    original.verify_frames(home, "large")
    original.verify_frames(home, "escaped")
    identity = {"uname": list(platform.uname()), "boot": boot, "affinity": [0],
                "nproc_all": subprocess.check_output(["nproc", "--all"], text=True).strip(),
                "disk": subprocess.check_output(["findmnt", "-nro", "MAJ:MIN", "--target", home], text=True).strip(),
                "cpuinfo": Path("/proc/cpuinfo").read_text().split("\n\n")[0],
                "uptime": subprocess.check_output(["uptime"], text=True).strip()}
    assert identity["disk"] == "8:0"
    original.write(out / "platform.json", identity)
    for index, order in enumerate(ORDER, 1):
        # Preserve all original pair orders and rotate the additional control.
        order = list(order)
        order.insert((index - 1) % 4, "move")
        profiles = PROFILES if index % 2 else tuple(reversed(PROFILES))
        for profile in profiles:
            for lane in order:
                binary = home / "bin" / LANES[lane]
                frames = home / "frames" / ("large" if profile == "plain" else profile)
                argv = ["taskset", "-c", "0", str(binary), "time", str(frames), str(REPEATS)]
                before = original.counters()
                result = subprocess.run(argv, capture_output=True, text=True, timeout=90, check=False)
                after = original.counters()
                original.write(out / f"{profile}-{index}-{lane}.json", {
                    "rotation": index, "order": order, "profile": profile, "lane": lane, "argv": argv,
                    "binary_sha256": pins[LANES[lane]]["binary_sha256"], "before": before, "after": after,
                    "exit": result.returncode, "stdout": result.stdout, "stderr": result.stderr,
                })
                result.check_returncode()
    original.write(out / "summary.json", replay(out))


def replay(out):
    identity = load(out / "platform.json")
    assert tuple(identity["uname"][i] for i in (0, 4, 2)) == original.HOST
    assert identity["boot"] == original.BOOT and identity["affinity"] == [0] and identity["disk"] == "8:0"
    pins = load(out / "build-pins.json")
    assert len({entry["binary_sha256"] for entry in pins.values()}) == 3
    semantic = load(out / "semantic.json")
    assert semantic["plain"]["output_sha256"] == "c98a0cd9500d920dfb07b040ffc7a90eb2628f82681eed196f2ea987db376e18"
    report = {"platform": identity, "build_pins_sha256": original.digest(out / "build-pins.json"),
              "semantic_sha256": original.digest(out / "semantic.json"), "profiles": {}}
    for profile in PROFILES:
        results = []
        raw = {}
        for index, order in enumerate(ORDER, 1):
            order = list(order)
            order.insert((index - 1) % 4, "move")
            group = {}
            for lane in order:
                path = out / f"{profile}-{index}-{lane}.json"
                row = load(path)
                raw[path.name] = original.digest(path)
                assert (row["profile"], row["rotation"], row["lane"], row["order"]) == (profile, index, lane, order)
                assert not row["stderr"] and row["exit"] == 0
                assert row["binary_sha256"] == pins[LANES[lane]]["binary_sha256"]
                assert row["argv"][:3] == ["taskset", "-c", "0"] and row["argv"][4::2] == ["time", str(REPEATS)]
                assert Path(row["argv"][3]).name == LANES[lane]
                assert Path(row["argv"][5]).name == ("large" if profile == "plain" else profile)
                elapsed = row["after"]["ticks"] - row["before"]["ticks"]
                steal = row["after"]["steal"] - row["before"]["steal"]
                assert elapsed > 0 and 0 <= steal / elapsed <= .01
                data = json.loads(row["stdout"])
                assert all(data[k] == semantic[profile][k] for k in ("output_sha256", "frames", "jobs"))
                assert data["frames"] == data["jobs"] == 64
                assert data["accepted"] == 64 * REPEATS and data["repeats"] == REPEATS
                assert data["cpu_ns"] > 0 and data["wall_ns"] > 0
                group[lane] = original.observed(data)
            results.append(group)
        cpu = [min(r["stock-a"][0], r["stock-b"][0]) - r["candidate"][0] for r in results]
        wall = [min(r["stock-a"][1], r["stock-b"][1]) - r["candidate"][1] for r in results]
        spread = [abs(r["stock-a"][0] - r["stock-b"][0]) for r in results]
        wins = sum(c > s and w > 0 for c, w, s in zip(cpu, wall, spread))
        report["profiles"][profile] = {
            "cpu_ms": {k: original.stats([r[k][0] for r in results]) for k in LANES},
            "wall_ms": {k: original.stats([r[k][1] for r in results]) for k in LANES},
            "cpu_saving_vs_faster_stock_ms": cpu, "wall_saving_vs_faster_stock_ms": wall,
            "duplicate_cpu_spread_ms": spread, "wins_over_jitter": wins,
            "candidate_cpu_saving_vs_move_ms": [r["move"][0] - r["candidate"][0] for r in results],
            "earned": profile == "plain" and statistics.median(cpu) >= THRESHOLD and wins >= 5,
            "raw_sha256": raw,
        }
    return report


if __name__ == "__main__":
    if sys.argv[1] == "run":
        run(*(Path(arg).resolve() for arg in sys.argv[2:]))
    elif sys.argv[1] == "replay":
        print(json.dumps(replay(Path(sys.argv[2])), indent=2, sort_keys=True, allow_nan=False))
    else:
        sys.exit("expected run or replay")
