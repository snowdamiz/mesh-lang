"""Copy completed native blocks and their hashes into the portable result packet."""

import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys
from tempfile import NamedTemporaryFile

from matrix import PACKET, QUEUE, SERIES, WORK, require, run, summarize, inputs
from report import scorecard

RELEASE_IDS = {**SERIES, "bulk": "dense-matched-topology-bulk-pair", "peer-setup": "dense-matched-topology-peer-setup"}


def copy(source, dest, checksum=None):
    expected = checksum or run.digest(source)
    if dest.exists():
        require(run.digest(dest) == expected, f"archive differs from source: {dest}")
        return
    dest.parent.mkdir(parents=True, exist_ok=True)
    with NamedTemporaryFile(dir=dest.parent, prefix=".copy-", delete=False) as stream:
        temp = Path(stream.name)
    try:
        shutil.copyfile(source, temp)
        require(run.digest(temp) == expected, f"copy differs from source: {dest}")
        temp.replace(dest)
    finally:
        temp.unlink(missing_ok=True)


def complete_blocks(source, dest):
    for marker in sorted((source / "blocks").glob("*/reconciled.json")):
        block = marker.parent
        target = dest / "blocks" / block.name
        if (target / marker.name).exists():
            continue
        copy(source / "series.json", dest / "series.json")
        copy(block / "order.json", target / "order.json")
        scored = run.read_json(marker)
        for phase_lane, score in scored.items():
            arm = next((block / phase_lane).iterdir())
            target_arm = target / phase_lane / arm.name
            copy(arm / "raw.tar.gz", target_arm / "raw.tar.gz", score["raw_sha256"])
            copy(arm / "request.json", target_arm / "request.json")
            copy(arm / "dispatch.json", target_arm / "dispatch.json")
        copy(marker, target / marker.name)
        print(f"{source.name} {block.name}: {len(scored)} raw archives checkpointed", flush=True)


def final_copy(kind, source, dest):
    receipt = QUEUE / f"{RELEASE_IDS[kind]}.released.json"
    require(receipt.is_file(), "controller release receipt is required before final copy")
    copy(receipt, dest / "controller-release.json")
    for path in sorted(source.rglob("*")):
        if path.is_file() and path.name != "transfer.tar":
            copy(path, dest / path.relative_to(source))
    if kind in SERIES:
        config, schedule, pins = inputs(kind)
        result = summarize(dest, config, schedule, pins)
        summary = dest / "replay.json"
        if not summary.exists():
            run.write_json(summary, result)
        require(run.read_json(summary) == result and result["qualified_rotations"] == 5,
                "replay has fewer than five qualified groups or differs from prior result")
        with (dest / "scorecard.md").open("w") as stream:
            scorecard(result, stream)
        if kind == "equal":
            roster = subprocess.check_output([sys.executable, "-B", PACKET / "peer-offline.py", dest])
            require(len(json.loads(roster)) >= 20, "missing equal topology process receipts")
            (dest / "peer-topology-replay.json").write_bytes(roster)
    elif kind == "bulk":
        data = subprocess.check_output([sys.executable, "-B", PACKET / "bulk.py", "replay", "--directory", dest])
        (dest / "replay.json").write_bytes(data)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=RELEASE_IDS)
    parser.add_argument("--final", action="store_true")
    args = parser.parse_args()
    source, dest = WORK / args.kind, PACKET / "evidence" / args.kind
    if args.final:
        final_copy(args.kind, source, dest)
    else:
        complete_blocks(source, dest)
