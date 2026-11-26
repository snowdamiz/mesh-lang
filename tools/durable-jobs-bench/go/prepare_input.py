#!/usr/bin/env python3
"""Export the exact frozen Linux regression batches for a native runner."""

import argparse
import hashlib
import runpy
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
harness = runpy.run_path(str(ROOT / "scripts/jobs-parity.py"))
PROFILES = {
    "noop": (64, "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"),
    "slow": (64, "42bb9e2be94c17b36110081430faf2647a04f3a3e0f2decf00e7c216fed462fd"),
    "sparse": (1, "20225faf6274696987b2192136f344d69be243f0a21dc9afad71e714c6ee08c5"),
    "short": (16, "ac0bcbf2465ea6d6807e8bfcdcfc3b1dacfa18b775e3f4c074257d66bfbba9d1"),
    "large": (64, "9d7f130f2cc690487582f73c9bea5fbb1f2661f8e416beb2f1574a6f868cc5c1"),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profile", choices=PROFILES)
    parser.add_argument("output", type=Path, help="new fixture directory")
    args = parser.parse_args()
    slots, expected = PROFILES[args.profile]
    settings = harness["json"].loads(harness["CONFIG"].read_text())
    batches = harness["inputs"](settings["workloads"][args.profile], slots, settings["seed"])
    args.output.mkdir()
    paths = harness["batch_files"](args.output, batches)
    digest = hashlib.sha256("".join(harness["digest"](path) for path in paths).encode()).hexdigest()
    if digest != expected:
        raise SystemExit(f"frozen input mismatch: {digest} != {expected}")
    print(digest)


if __name__ == "__main__":
    main()
