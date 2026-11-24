"""Bind the unchanged dense plan to the controller's recovered D2 SETUP."""

import argparse
from importlib import import_module
import json
from pathlib import Path
import sys


HERE = Path(__file__).resolve().parent
RECOVERY = HERE.parent / "2026-11-24-exit-confirmed-bulk-d2/recovery"
RECOVERY_SEAL = "bfb7b78df767dbde23318c950e4fa8936d8cfd5a16544f6a7f9e5d0e6e688bec"
DENSE_SEAL = "5519eab2776b8dc853f9db7ac60a46ead2278db492b6f7ba2848b53060f4dde9"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "timing", "replay"))
    parser.add_argument("groups", type=int, choices=range(1, 7), nargs="*")
    args = parser.parse_args()
    dense = import_module("native-controller")
    sys.path.insert(0, str(RECOVERY))
    try:
        recovery = import_module("recover")
    finally:
        sys.path.remove(str(RECOVERY))
    run = dense.run
    dense.require(run.digest(dense.REQUEST) == DENSE_SEAL, "dense request seal changed")
    packet = dense.request()
    recovery_packet, baseline, _ = recovery.inputs()
    recovery_request = dense.QUEUE / f"{recovery.SETUP_ID}.json"
    dense.require(run.digest(recovery_request) == RECOVERY_SEAL
                  and run.read_json(recovery_request) == recovery_packet,
                  "recovery request seal or contents changed")
    release = dense.QUEUE / f"{recovery.SETUP_ID}.released.json"
    dense.require(run.read_json(release)["exit_status"] == 0, "recovered D2 SETUP did not release successfully")
    recovered = recovery.qualified(baseline)["lanes"]["r2-d2"]
    launcher = recovered["inventory"]["engines"]["mesh"]["argv"]
    assignment, = (arg for arg in launcher if arg.startswith("COMBINED_RUNNER_ARTIFACTS="))
    candidate = {key: recovered["runner_seal"][key] for key in ("elf", "manifest")}
    candidate["path"] = assignment.split("=", 1)[1]
    config = dense.configurations(candidate)
    blocks = list(dense.schedule())
    pins = {**packet["local_sha256"], "native_request": DENSE_SEAL,
            "d2_setup": run.digest(recovery.QUALIFICATION), "d2_recovery_request": RECOVERY_SEAL,
            "d2_recovery_release": run.digest(release), "recovery_binder": run.digest(Path(__file__))}
    if args.action == "check":
        for label in config["lanes"]:
            dense.stages(config, label, run.REMOTE / label)
        print(json.dumps(dict(candidate=candidate, rotations=[b["group"] for b in blocks],
                              lanes=list(config["lanes"]), pins=pins), sort_keys=True, indent=2))
        return 0
    dense.attach_matrix()
    if args.action == "timing":
        dense.grant()
        dense.require(args.groups in ([1, 2], [3, 4, 5, 6]), "use the sealed pilot or remaining groups")
        return dense.dispatch(config, blocks, pins, args.groups)
    report = dense.summarize(dense.RESULTS, config, blocks, pins)
    print(json.dumps(report, indent=2, sort_keys=True, allow_nan=False))
    return bool(report["missing"])


if __name__ == "__main__":
    sys.exit(main())
