"""Use the sealed V2 driver with two reversed short rotations."""

import importlib


matrix = importlib.import_module("interleave-v2")
original_plan = matrix.plan


def plan(profiles, lanes):
    for block in original_plan(profiles, lanes):
        if block["group"] in (2, 4):
            block = dict(block, order=block["order"][::-1])
        yield block


if __name__ == "__main__":
    matrix.plan = plan
    raise SystemExit(matrix.main())
