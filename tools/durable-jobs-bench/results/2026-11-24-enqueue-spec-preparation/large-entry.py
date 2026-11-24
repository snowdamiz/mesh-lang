#!/usr/bin/env python3
"""Use the work_mem launcher with the candidate as its sole broker input."""

import json
from pathlib import Path
import runpy
import sys

HERE = Path(__file__).resolve().parent


def main():
    engine, work_mem, launcher, *args = sys.argv[1:]
    if engine != "mesh" or work_mem != "16":
        raise ValueError("expected the ordinary 16 MiB Mesh lane")
    launcher = Path(launcher).resolve(strict=True)
    expected = json.loads((HERE / "launch-pins.json").read_text())
    if str(launcher) != expected["launcher"]:
        raise ValueError("unexpected ordinary adapter")
    sys.path.insert(0, str(launcher.parent))
    import assembly

    configure = assembly.configuration

    def select_broker(*args):
        stock, policy = configure(*args)
        candidate = json.loads((HERE / "broker-pins.json").read_text())
        if policy != "external" or dict(stock, **{k: candidate[k] for k in expected["broker_fields"]}) != candidate:
            raise ValueError("candidate altered the ordinary assembly outside the broker pins")
        return candidate, policy

    assembly.configuration = select_broker
    stock_entry = Path(expected["stock_entry"])
    sys.path.insert(0, str(stock_entry.parent))
    runpy.run_path(str(stock_entry))["invoke"]("mesh", 16, launcher, args)


if __name__ == "__main__":
    main()
