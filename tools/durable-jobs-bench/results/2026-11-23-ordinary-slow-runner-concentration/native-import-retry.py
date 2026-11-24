"""Retry the exact slow matrix with explicit imports through the private staging directory."""

import argparse
import json
from pathlib import Path
import subprocess
import sys

import native


ID = native.IDENT + "-import-retry"
LAUNCHER = native.HERE / "import-retry/native-mesh.sh"
REQUEST = native.HERE / "request-import-retry.json"
OUT = native.HERE / "evidence/native-import-retry"
STOCK = native.staged


def staged(config, label, remote):
    additions, inventory = STOCK(config, label, remote)
    if config["lanes"][label]["engine"] == "mesh":
        additions = [LAUNCHER if path.name == LAUNCHER.name else path for path in additions]
        inventory["engines"]["mesh"]["verified_files"][str(remote / LAUNCHER.name)] = native.run.digest(LAUNCHER)
    return additions, inventory


def pins():
    return {str(path.relative_to(native.ROOT)): native.run.digest(path) for path in
            (*native.DRIVERS, LAUNCHER, Path(__file__).resolve())}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "dispatch", "replay"))
    args = parser.parse_args()
    native.IDENT = ID
    native.staged = staged
    config = native.freeze("two")
    hashes = pins()
    original = native.run.read_json(native.HERE / "request-stock-peers.json")
    native.require(all(hashes[path] == digest for path, digest in original["local_sha256"].items()),
                   "original workload, scorer or launcher changed")
    packet = native.run.read_json(REQUEST)
    native.require(packet["local_sha256"] == hashes and packet["id"] == ID,
                   "import retry request differs from the frozen inputs")
    if args.action == "check":
        subprocess.run(["bash", "-n", LAUNCHER], check=True)
        for label in config["lanes"]:
            staged(config, label, native.run.REMOTE / label)
        print("Verified original slow work, bins, scorer and order with the explicit loader")
        return 0
    if args.action == "replay":
        print(json.dumps(native.replay(OUT, config, hashes), indent=2, sort_keys=True, allow_nan=False))
        return 0
    return native.dispatch(OUT, config, hashes, REQUEST, [1, 2])


if __name__ == "__main__":
    sys.exit(main())
