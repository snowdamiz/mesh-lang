"""Replay D2 timing with its preselected order reconstructed from the sealed map."""

from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path
import runpy
import shlex
import sys


STATE = Path(__file__).resolve().parent
PROOF = "ordinary-hot-checkout-buildfree"
spec = spec_from_file_location("d2_retained_proof", STATE / "d2-native-recovery.py")
proof = module_from_spec(spec)
spec.loader.exec_module(proof)
d2 = proof.d2
run = proof.run
d2.seal(proof.FREEZE)
d2.interleave.plan = d2.plan
d2.interleave.replay = d2.replay


def qualified():
    config = proof.accepted_configuration()
    lanes = config["lanes"]
    run.require(set(lanes) == set(d2.LABELS), "the D2 proof has different controls")
    return dict(config, lanes={label: lanes[label] for label in d2.LABELS})


if __name__ == "__main__":
    command = [sys.executable, "-B", __file__]
    series = proof.LANE / "evidence" / d2.SERIES
    if sys.argv[1:] == ["summarize"]:
        sys.exit(d2.interleave.summarize(series))
    if sys.argv[1:] == ["check"]:
        configuration = qualified()
        schedule = list(d2.plan(configuration["profiles"], configuration["lanes"]))
        plan = run.read_json(STATE / "requests" / "ordinary-hot-checkout.json")["timing"]["order"]
        run.require(schedule == plan, "the D2 control order changed")
        print("D2 timing retry reproduces all five sealed rotations")
        sys.exit(0)
    if len(sys.argv) == 3 and sys.argv[1] == "timing":
        d2.grant("TIMING", d2.SERIES, 60)
        sys.exit(d2.interleave.dispatch(series, qualified(), [int(sys.argv[2])]))
    run.require(len(sys.argv) == 1, "expected check, summarize, timing GROUP or no arguments")
    saved = run.read_json(STATE / "controller-receipts" / PROOF / "request.json")
    saved["timing"]["exact_group_commands"] = [shlex.join([*command, "timing", str(n)]) for n in range(1, 6)]
    saved["timing"]["summarize_command"] = shlex.join([*command, "summarize"])
    driver = runpy.run_path(str(STATE / "hot-checkout-slots.py"))
    sys.exit(driver["timing"](saved, [1, 2, 3, 4, 5]))
