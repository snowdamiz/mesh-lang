"""Reduce all saved arms. Timing deltas pair reuse with both stock controls."""
import argparse
from collections import defaultdict
import json
from pathlib import Path
import statistics


def describe(xs):
    return {"n": len(xs), "median": statistics.median(xs), "min": min(xs), "max": max(xs)}


def project(d):
    n = d["total"]
    work = d["work"]
    return {
        "full_cpu_us_per_actor": (d["full_user_s"] + d["full_system_s"]) * 1e6 / n,
        "full_user_us_per_actor": d["full_user_s"] * 1e6 / n,
        "full_system_us_per_actor": d["full_system_s"] * 1e6 / n,
        "full_wall_us_per_actor": d["full_wall_s"] * 1e6 / n,
        "body_wall_us_per_actor": work["body_wall_s"] * 1e6 / n,
        "minor_faults_per_actor": d["full_minor_faults"] / n,
        "max_rss_kib": d["max_rss_bytes"] / 1024,
    }


def memory(d):
    phases = d["phases"]
    stamps = ("baseline", "first_live", "first_drained", "last_live", "last_drained")
    result = {}
    for label, phase in zip(stamps, phases, strict=True):
        result[label + "_rss_kib"] = phase["rss"] / 1024
        result[label + "_rss_delta_kib"] = (phase["rss"] - phases[0]["rss"]) / 1024
        result[label + "_vm_delta_kib"] = (phase["vm"] - phases[0]["vm"]) / 1024
        result[label + "_os_threads"] = phase["threads"]
    result["max_rss_kib"] = d["max_rss_bytes"] / 1024
    return result


def result(path):
    plan = json.loads((path / "plan.json").read_text())
    cases = defaultdict(lambda: defaultdict(dict))
    for round_n, hold, width, workers, label in plan["plan"]:
        stem = f"{round_n:02d}-park{hold}-live{width:02d}-threads{workers}-{label}"
        f = path / (stem + ".json")
        row = json.loads(f.read_text())
        assert row["exit_code"] == 0 and row["work"]["completed"] == row["total"] == plan["total"]
        cases[f"{'parked' if hold else 'immediate'}/{width}/{workers}"][round_n][label] = row

    metrics = {}
    for case, rounds in cases.items():
        labels = defaultdict(list)
        differences = defaultdict(list)
        spreads = defaultdict(list)
        for rows in rounds.values():
            if plan["mode"] == "counts":
                for label, row in rows.items():
                    calls = row["calls"][0]
                    assert calls["65536"]["reserve"] == 0
                    counted = calls["524288"]
                    assert counted["reserve"] == counted["protect"] == counted["unmap"]
                    assert counted["live"] == 0
                    if label == "stock-a":
                        assert counted["reserve"] == row["total"] and counted["reset"] == 0
                    else:
                        assert counted["reserve"] < row["total"]
                        retained_on_thread_exit = counted["reset"] - (row["total"] - counted["reserve"])
                        assert 0 < retained_on_thread_exit <= 8 * row["workers"]
                    labels[label].append(dict(calls, idle_at_worker_exit=(
                        counted["reset"] - (row["total"] - counted["reserve"]))))
                continue

            projector = memory if plan["mode"] == "memory" else project
            block = {label: projector(row) for label, row in rows.items()}
            for label, values in block.items():
                labels["reuse" if label == "reuse" else "stock"].append(values)
            a, b = block["stock-a"], block["stock-b"]
            for key in a:
                spreads[key].append(abs(a[key] - b[key]))
                if "reuse" in block:
                    differences[key].append((a[key] + b[key]) / 2 - block["reuse"][key])

        if plan["mode"] == "counts":
            metrics[case] = {label: rows[0] for label, rows in labels.items()}
            continue
        metrics[case] = {
            "arms": {label: {key: describe([arm[key] for arm in arms]) for key in arms[0]}
                     for label, arms in labels.items()},
            "paired_stock_minus_reuse": {key: describe(d) | {"positive": sum(x > 0 for x in d)}
                                         for key, d in differences.items()},
            "within_round_stock_control_spread": {key: describe(d) for key, d in spreads.items()},
        }
    return metrics


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("directories", type=Path, nargs="+")
    a = p.parse_args()
    print(json.dumps({path.name: result(path) for path in a.directories}, indent=2, sort_keys=True))
