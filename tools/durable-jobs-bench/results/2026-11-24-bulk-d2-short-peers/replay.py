"""Measure the exact qualified bulk+D2 runner against all stock short peers."""

from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path
import sys


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
SOURCE = ROOT / "target/ordinary-hot-checkout/bulk-v2/native/dispatch.py"
spec = spec_from_file_location("sealed_bulk_d2", SOURCE)
d2 = module_from_spec(spec)
spec.loader.exec_module(d2)
matrix = d2.interleave
run = d2.run
FREEZE = "7b4d1e9831ff46f63ee1a955232989fb505899245804841ead8c8ea4f2a95cc5"
LANES = {"bulk", "bulk-d2", "protected-mesh", "oban", "river", "apalis"}
normal_validate, normal_plan, normal_replay = matrix.validate, matrix.plan, matrix.replay


def validate(config):
    normal_validate(config)
    run.require(config["profiles"] == ["short"] and config["cpus"] == [0, 1]
                and set(config["lanes"]) == LANES, "use the exact six-lane short comparison")


def plan(profiles, lanes):
    for block in normal_plan(profiles, lanes):
        if block["group"] in (2, 4):
            block = dict(block, order=list(reversed(block["order"])))
        yield block


def replay(path, config, block):
    result = normal_replay(path, config, block)
    for phase, label, folder in matrix.arms(path, config, block):
        if label not in {"bulk", "bulk-d2"}:
            continue
        proof = d2.audit(folder, config, phase, label)
        if phase == "measured":
            result["scores"][label].update({name: proof[name]
                                            for name in ("checkout_probes", "app_writes", "bulk_calls")})
    return result


if __name__ == "__main__":
    d2.verify_freeze(FREEZE)
    if sys.argv[1:] == ["check"]:
        config = run.read_json(HERE / "short.json")
        validate(config)
        print("Qualified two-CPU bulk+D2, bulk and stock peer inventory verified offline")
        sys.exit(0)
    matrix.RESULTS = HERE / "evidence"
    matrix.validate, matrix.plan, matrix.replay = validate, plan, replay
    sys.exit(matrix.main())
