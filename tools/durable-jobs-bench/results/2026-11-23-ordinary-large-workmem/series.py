#!/usr/bin/env python3
"""Use the unchanged native scheduler and score, plus require work_mem evidence."""

import argparse
from pathlib import Path
import sys

from freeze import LABELS, LANE, digest, read, verify
from proof import stock_matrix, native, matched_settings
from setup import ID, REQUESTS, grant

SERIES = "ordinary-large-workmem-matched"
STOCK_REPLAY = stock_matrix.replay


def replay(path, config, block):
    result = STOCK_REPLAY(path, config, block)
    diagnostics = {}
    for phase, label, folder in stock_matrix.arms(path, config, block):
        sidecar = native(folder, config, phase, label, result["scores"][label] if phase == "measured" else None)
        diagnostics.setdefault(phase, {})[label] = sidecar
    for phase in block["phases"]:
        matched_settings([row for label, row in diagnostics[phase].items() if label.startswith("ordinary")])
    return dict(result, postgres=diagnostics)


def require_setup(checksum):
    released = (REQUESTS / "STATUS.md").read_text()
    if not any(line.startswith(f"RELEASED {ID} ") and "exit 0" in line for line in released.splitlines()):
        raise ValueError("the controller has not released a successful proof slot")
    for path in (LANE / "evidence").glob(f"{ID}-*/setup-status.json"):
        status = read(path)
        if (status["exit_code"] == 0 and status["freeze_sha256"] == checksum
                and status["arms"].keys() == LABELS.keys()
                and all(row["qualified"] for row in status["arms"].values())):
            return
    raise ValueError("a successful proof for all lanes of this exact freeze is required")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("dispatch", "summarize"))
    parser.add_argument("freeze", type=Path)
    parser.add_argument("sha256")
    parser.add_argument("--group", type=int, choices=range(1, 6), action="append")
    args = parser.parse_args()
    if digest(args.freeze / "freeze.json") != args.sha256:
        raise ValueError("wrong native freeze")
    verify(args.freeze)
    stock_matrix.replay = replay
    if args.action == "summarize":
        return stock_matrix.summarize(LANE / "evidence" / SERIES)
    if not args.group:
        parser.error("pass one or more explicitly assigned groups")
    require_setup(args.sha256)
    grant("TIMING", SERIES)
    return stock_matrix.dispatch(LANE / "evidence" / SERIES, read(args.freeze / "large.json"), args.group)


if __name__ == "__main__":
    sys.exit(main())
