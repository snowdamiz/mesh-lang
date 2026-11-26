#!/usr/bin/env python3
"""Run one six-lane comparison through the existing frozen native timing driver."""
import importlib
from pathlib import Path
import sys
import tarfile

from proof import check

HERE = Path(__file__).resolve().parent
PACKET = HERE.parent
PRIOR = PACKET.parents[1] / "2026-11-23-bulk-enqueue-v2-short"
sys.path.insert(0, str(PRIOR / "replay"))
matrix = importlib.import_module("interleave-v2")
run = matrix.run
SERIES = "bulk-v2-completion-writer-exit-r2"
matrix.RESULTS = PACKET / "evidence" / "native-timing"
base_replay = matrix.replay

ORDERS = (
    ("bulk", "writer-off", "river", "writer-zero", "oban", "apalis"),
    ("river", "apalis", "writer-zero", "bulk", "writer-off", "oban"),
    ("writer-zero", "writer-off", "apalis", "river", "bulk", "oban"),
)


def plan(profiles, lanes):
    assert tuple(profiles) == ("short",) and set(lanes) == set(ORDERS[0])
    orders = (ORDERS[0], ORDERS[0][::-1], ORDERS[1], ORDERS[1][::-1], ORDERS[2])
    for group, order in enumerate(orders, 1):
        yield dict(group=group, profile="short", phases=["measured"], order=list(order))


def replay(path, config, block):
    scored = base_replay(path, config, block)
    for phase, label, folder in matrix.arms(path, config, block):
        if label in {"writer-off", "writer-zero"}:
            lane = config["lanes"][label]
            with tarfile.open(folder / "raw.tar.gz") as archive:
                try:
                    stats = check(lambda name: run.file_json(archive, "adapter/" + name),
                                  label.removeprefix("writer-"), lane["runner_seal"], native=True)
                except AssertionError as error:
                    raise ValueError(f"{label} failed the split-executable, bulk writer or checkout proof") from error
            if phase == "measured":
                scored["scores"][label].update(stats)
    scores = scored["scores"]
    scored["writer_zero_vs_writer_off"] = {
        metric: dict(ratio=scores["writer-zero"][metric] / scores["writer-off"][metric],
                     saved_ms=128000 * (1 / scores["writer-off"][metric] - 1 / scores["writer-zero"][metric]))
        for metric in ("submit_jobs_per_s", "commit_jobs_per_s")
    }
    return scored


matrix.plan = plan
matrix.replay = replay

if __name__ == "__main__":
    raise SystemExit(matrix.main())
