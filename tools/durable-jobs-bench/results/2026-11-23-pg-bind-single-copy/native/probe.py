#!/usr/bin/env python3
"""Run controller-owned Linux AMD64 Bind allocation controls without builds."""
import json
from pathlib import Path
import subprocess
import sys

probe, evidence = map(Path, sys.argv[1:])
arms = ("stock", "stock-b", "optimized-a", "optimized-b")
shapes = ("short", "scalar", "mixed")
orders = ((0, 1, 3, 2), (1, 2, 0, 3), (2, 3, 1, 0), (3, 0, 2, 1))
orders += tuple(tuple(reversed(order)) for order in orders)
metrics = ("allocs", "reallocs", "peak_live_requested", "frame_bytes",
           "explicit_bind_body_copy_bytes")
rows = []
for group, order in enumerate(orders, 1):
    for shape in shapes:
        observed = {}
        for index in order:
            arm = arms[index]
            executable = probe / ("probe-b" if arm.endswith("-b") else "probe-a")
            stem = evidence / f"{group:02d}-{shape}-{arm}"
            with stem.with_suffix(".stdout").open("xb") as stdout, stem.with_suffix(".stderr").open("xb") as stderr:
                subprocess.run([executable, arm, shape, "alloc", evidence / "unused"],
                               stdout=stdout, stderr=stderr, check=True, timeout=20)
            fields = dict(line.split("=") for line in stem.with_suffix(".stdout").read_text().splitlines())
            assert fields["iterations"] == "1"
            observed[arm] = {key: int(fields[key]) for key in metrics}
            rows.append({"group": group, "shape": shape, "arm": arm, **observed[arm]})
        stock, candidate = observed["stock"], observed["optimized-a"]
        assert stock == observed["stock-b"] and candidate == observed["optimized-b"], observed
        assert stock["frame_bytes"] == candidate["frame_bytes"], observed
        assert 0 < stock["explicit_bind_body_copy_bytes"] and candidate["explicit_bind_body_copy_bytes"] == 0
        assert stock["peak_live_requested"] > candidate["peak_live_requested"], observed
        assert stock["allocs"] + stock["reallocs"] > candidate["allocs"] + candidate["reallocs"], observed
(evidence / "allocation.json").write_text(json.dumps(rows, indent=2, sort_keys=True) + "\n")
