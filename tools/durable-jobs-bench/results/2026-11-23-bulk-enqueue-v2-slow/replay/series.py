"""Use canonical native replay with both ordinary runtime generations."""

import importlib.util
from pathlib import Path
import sys
import tarfile


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(ROOT / "tools/durable-jobs-bench"))
BASE = ROOT / "tools/durable-jobs-bench/results/2026-11-23-bulk-enqueue-v2-short/replay/interleave-v2.py"
spec = importlib.util.spec_from_file_location("bulk_v2_matrix", BASE)
matrix = importlib.util.module_from_spec(spec)
spec.loader.exec_module(matrix)
run = matrix.run
base_validate = matrix.validate
base_plan = matrix.plan
base_replay = matrix.replay
CONTROLS = {"v1-scalar", "v1-bulk"}
V2 = {"bulk", "scalar"}


def validate(config):
    lanes = config["lanes"]
    run.require(config["profiles"] == ["slow"] and config["cpus"] == [0, 1]
                and lanes.keys() == V2 | CONTROLS | {"oban", "river", "apalis"},
                "expected the exact seven-lane ordinary slow comparison")
    base_validate(dict(config, lanes={label: lane for label, lane in lanes.items() if label not in CONTROLS}))
    base_validate(dict(config, lanes={label: lane for label, lane in lanes.items() if label in CONTROLS}))


def plan(profiles, lanes):
    blocks = list(base_plan(profiles, lanes))
    blocks[1] = dict(blocks[1], order=blocks[0]["order"][::-1])
    yield from blocks


def replay(path, config, block):
    result = base_replay(path, config, block)
    for _, label, folder in matrix.arms(path, config, block):
        if label not in CONTROLS:
            continue
        with tarfile.open(folder / "raw.tar.gz", "r:gz") as bundle:
            writer = run.file_json(bundle, "adapter/producer-writer.json")
            ledger = run.file_json(bundle, "adapter/ledger.json")
        bulk = label == "v1-bulk"
        run.require(writer["enabled"] == bulk and (writer["bulk"] > 0 if bulk else writer["bulk"] == 0),
                    "V1 writer differed from its pinned lane")
        run.require(len({row["settled_xid"] for row in ledger["jobs"]}) == 512,
                    "V1 control did not keep individual terminal XIDs")
    return result


if __name__ == "__main__":
    matrix.RESULTS = HERE / "evidence"
    matrix.validate = validate
    matrix.plan = plan
    matrix.replay = replay
    raise SystemExit(matrix.main())
