"""Recompute the three slow64 results using the frozen repository scorer."""

from collections import Counter, defaultdict
import csv
import gzip
from hashlib import file_digest, sha256
import json
from pathlib import Path
import runpy

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
SLOW_INPUT = "42bb9e2be94c17b36110081430faf2647a04f3a3e0f2decf00e7c216fed462fd"
MODES = ("ordinary", "through_shutdown")


def digest(path):
    with path.open("rb") as file:
        return file_digest(file, "sha256").hexdigest()


def summary_digest(result):
    return sha256(json.dumps(result, sort_keys=True, separators=(",", ":"),
                             allow_nan=False).encode()).hexdigest()


def holds(intervals):
    changes = Counter()
    for start, end in intervals:
        assert 0 <= start < end
        changes[start] += 1
        changes[end] -= 1
    active, peak = 0, 0
    for stamp in sorted(changes):
        active += changes[stamp]
        assert 0 <= active <= 64
        peak = max(peak, active)
    assert active == 0
    return peak


def score(harness, settings, groups, label, mode):
    trials = []
    for index, group in enumerate(groups):
        for arm, engine in ((label, "mesh"), ("oban", "oban")):
            record = group["measured"][arm]
            row = dict(record["metrics"], pair=index, engine=engine)
            if mode == "through_shutdown":
                row.update(record["shutdown"])
            trials.append(row)
    harness["validate_pairs"](trials, len(groups), 64)
    summary = harness["profile_summary"](
        trials, "slow", settings["workloads"]["slow"], settings,
        1, 64, ("mesh", "oban"), True, True, None,
    )
    misses = [f"{scope}.{metric}" for scope, metrics in summary.items()
              for metric, rule in metrics.items() if isinstance(rule, dict)
              and (rule.get("within_limit") is False or rule.get("within_budget") is False)]
    return summary, {"misses": misses, "summary_sha256": summary_digest(summary)}


def main():
    record = json.loads((HERE / "slow64-receipts.json").read_text())
    assert record["schema"] == 1
    for path, expected in record["scorer_sha256"].items():
        assert digest(ROOT / path) == expected, path
    harness = runpy.run_path(str(ROOT / "scripts/jobs-parity.py"))
    settings = json.loads((ROOT / "scripts/jobs-parity-profiles.json").read_text())
    intervals = defaultdict(list)
    archive = HERE / "committed-holds.csv.gz"
    assert digest(archive) == record["intervals_sha256"]
    with gzip.open(archive, "rt", newline="") as source:
        for suite, group, label, start, end in csv.reader(source):
            intervals[suite, group, label].append((int(start), int(end)))

    for name, series in record["series"].items():
        groups = series["groups"]
        assert len(groups) == series["planned_groups"]
        for group in groups:
            assert set(group["order"]) == group["measured"].keys()
            for label, arm in group["measured"].items():
                row = arm["metrics"]
                samples = intervals.pop((name, group["name"], label))
                assert row["completed"] == arm["ordered_distinct_xid_write_pairs"] == len(samples) == 512
                assert row["input_sha256"] == SLOW_INPUT
                assert arm["overlap"] == holds(samples)
                assert all(value >= row[metric] for metric, value in arm["shutdown"].items())
            for label in series["mesh_roles"]:
                for mode in MODES:
                    _, replayed = score(harness, settings, [group], label, mode)
                    assert replayed == group["scores"][label][mode], (name, group["name"], label, mode)
        for label, role in series["mesh_roles"].items():
            occupancies = [group["measured"][label]["overlap"] for group in groups]
            print(f"{name} {label} ({role}) committed holds {occupancies}")
            for mode in MODES:
                summary, replayed = score(harness, settings, groups, label, mode)
                assert replayed == series["aggregate"][label][mode], (name, label, mode)
                mesh, oban = summary["mesh"], summary["oban"]
                rates = (side["jobs_per_second"]["median"] for side in (mesh, oban))
                rss = (side["peak_rss_bytes"]["median"] / 2**20 for side in (mesh, oban))
                print(f"  {mode}: %.2f / %.2f jobs/s; %.2f / %.2f MiB; %s"
                      % (*rates, *rss, replayed["misses"]))
    assert not intervals


if __name__ == "__main__":
    if not __debug__:
        raise RuntimeError("Run without Python optimization to enable verification")
    main()
