#!/usr/bin/env python3
"""Run the sealed short16 timing driver with the pinned closure/eight-lane amendment.

The original driver, intake and interleave-v2 files are read unchanged from the
private checkout and verified by SHA-256. Only these pinned substitutions apply:
the TIMING grant becomes ``<ID>-closure-retry``, the overlay SETUP release is
also required, interleave-v2's six-lane bound becomes the exact frozen eight
short16 lanes, and every bound mesh lane must use a replay-closure-retry adapter.
"""

import hashlib
import inspect
from pathlib import Path
import sys
from textwrap import dedent

ROOT = Path(__file__).resolve().parents[5]
REPLAY = ROOT / "target/short16-fused-refill/source/tools/durable-jobs-bench/results/2026-11-24-short16-fused-refill/replay"
DRIVER = REPLAY / "controller-timing.py"
INTERLEAVE = REPLAY.parents[1] / "2026-11-23-bulk-enqueue-v2-short/replay/interleave-v2.py"
PINS = {
    DRIVER: "2f68b1303d4a3e5cc8dc19e6d0e91a9bd8a56d9857a8b789d34357fe013182ab",
    REPLAY / "intake.py": "95129282841b8ef6134c5ddcc3ec42d4bd943137d2231b7bf230767ff21b03e4",
    INTERLEAVE: "d30834fa1ca18030edd5df0de5d760ea1952d2b84aa05fb3eae9d2af13825c52",
}
LANES = sorted(("public-group", "group-a", "group-b", "fused-a", "fused-b", "river", "oban", "apalis"))
GRANT = '    assert (kind, key) == ("TIMING", ID), f"missing controller TIMING grant for {ID}"\n'
SETUP = '    assert setup["id"] == ID and setup["exit_status"] == 0\n'
CONFIG = '    config = qualification["configuration"]\n'
IMPORT = 'matrix.ATTEMPT_ERRORS += (AssertionError,)\n'
BOUND = '    run.require(2 <= len(config["lanes"]) <= 6, "specify between two and six lanes")\n'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def amended_source():
    for path, expected in PINS.items():
        assert digest(path) == expected, f"sealed timing input changed: {path}"
    source = DRIVER.read_text()
    for anchor in (GRANT, SETUP, CONFIG, IMPORT):
        assert source.count(anchor) == 1, f"sealed driver anchor changed: {anchor!r}"
    source = source.replace(GRANT, GRANT.replace('("TIMING", ID)', '("TIMING", f"{ID}-closure-retry")'))
    source = source.replace(SETUP, SETUP + (
        '    overlay = read(requests / f"{ID}-closure-retry.released.json")\n'
        '    assert overlay["id"] == f"{ID}-closure-retry" and overlay["exit_status"] == 0\n'))
    source = source.replace(CONFIG, CONFIG + (
        '    for lane in config["lanes"].values():\n'
        '        if lane["engine"] == "mesh":\n'
        '            assert "/replay-closure-retry/native-run" in lane["inventory"]["engines"]["mesh"]["argv"][-2], '
        '"mesh lane is not bound to the closure-retry adapter"\n'))
    return source.replace(IMPORT, IMPORT + "_closure_retry_validate(matrix)\n")


def eight_lane_validate(matrix):
    """Replace only the lane-count bound of the unchanged interleave-v2 validator."""
    original = matrix.validate
    source = dedent(inspect.getsource(original))
    assert source.count(BOUND.strip()) == 1, "interleave-v2 lane bound changed"
    replacement = 'run.require(sorted(config["lanes"]) == %r, "use exactly the frozen eight short16 lanes")' % LANES
    source = source.replace(BOUND.strip(), replacement)
    scope = {}
    exec(compile(source, original.__code__.co_filename, "exec"), original.__globals__, scope)
    matrix.validate = scope["validate"]


if __name__ == "__main__":
    sys.path.insert(0, str(REPLAY))
    sys.argv[0] = str(DRIVER)
    exec(compile(amended_source(), str(DRIVER), "exec"),
         dict(__file__=str(DRIVER), __name__="__main__", _closure_retry_validate=eight_lane_validate))
