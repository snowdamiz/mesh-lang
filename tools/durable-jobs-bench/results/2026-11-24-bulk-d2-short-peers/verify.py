"""Reconcile the six-lane short series from archived Linode evidence."""

import argparse
import gzip
from pathlib import Path
from shutil import copy2, copyfileobj
import subprocess
import sys
from tempfile import TemporaryDirectory


PACKET = Path(__file__).resolve().parent
ROOT = PACKET.parents[3]
SERIES = PACKET.name
FROZEN = PACKET.parent / "2026-11-23-ordinary-hot-checkout-bulk-v2/sealed-inputs"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify inputs before timing completes")
    args = parser.parse_args()
    with TemporaryDirectory(dir=ROOT / "target", prefix="short-peers-replay-") as tmp:
        root = Path(tmp)
        suite = root / "tools/durable-jobs-bench"
        replay_dir = suite / "results" / SERIES
        replay_dir.mkdir(parents=True)
        for name in ("run.py", "verify.py", "worker.py"):
            (suite / name).symlink_to(ROOT / "tools/durable-jobs-bench" / name)
        for source in sorted(path for path in FROZEN.rglob("*") if path.is_file()):
            target = root / source.relative_to(FROZEN)
            target.parent.mkdir(parents=True, exist_ok=True)
            if target.name.endswith(".log.gz"):
                with gzip.open(source, "rb") as stream, target.with_suffix("").open("xb") as output:
                    copyfileobj(stream, output)
            else:
                copy2(source, target)
        (root / "target/ordinary-hot-checkout/bulk-v2/tmp").mkdir()
        for name in ("replay.py", "short.json"):
            copy2(PACKET / name, replay_dir / name)
        (replay_dir / "evidence").symlink_to(PACKET / "evidence", target_is_directory=True)
        command = [sys.executable, "-B", str(replay_dir / "replay.py")]
        if args.check:
            subprocess.run([*command, "check"], cwd=ROOT, check=True)
            return
        replay = subprocess.check_output([*command, "summarize", SERIES], cwd=ROOT)
        if replay != (PACKET / "evidence" / SERIES / "summary.json").read_bytes():
            sys.exit("Recomputed summary differs from the archived short series")
    print("Five native six-lane rotations reproduced from raw evidence")


if __name__ == "__main__":
    main()
