"""Dispatch frozen slow groups 3–6 under a new grant, retaining the original pilot."""

import argparse
from importlib import import_module
from pathlib import Path
import sys

import native


retry = import_module("native-import-retry")
ID = "ordinary-slow-runner-concentration-followup"
REQUEST = native.HERE / "request-followup.json"
GROUPS = [3, 4, 5, 6]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "dispatch"))
    args = parser.parse_args()
    doc = native.run.read_json(REQUEST)
    native.require(doc["id"] == ID and doc["groups"] == GROUPS and doc["ready"]
                   and doc["cpus"] == [0, 1], "wrong slow continuation request")
    for relative, expected in doc["local_sha256"].items():
        native.require(native.run.digest(native.ROOT / relative) == expected,
                       f"frozen continuation input changed: {relative}")
    native.require(REQUEST.read_bytes() == (native.QUEUE / f"{ID}.json").read_bytes(),
                   "slow continuation request and inbox disagree")
    release = native.run.read_json(retry.OUT / "release.json")
    native.require(release["id"] == retry.ID and release["groups"] == [1, 2]
                   and release["exit_status"] == 0, "the first two groups must have a successful release")
    native.IDENT = ID
    native.staged = retry.staged
    config, pins = native.freeze("two"), retry.pins()
    pilot = native.replay(retry.OUT, config, pins)
    native.require(not {1, 2} & set(pilot["missing"]), "both frozen pilot outcomes must reconcile")
    if args.action == "check":
        print("Both pilot blocks replayed. Frozen continuation groups are 3, 4, 5 and 6.")
        return 0
    return native.dispatch(retry.OUT, config, pins, REQUEST, GROUPS)


if __name__ == "__main__":
    sys.exit(main())
