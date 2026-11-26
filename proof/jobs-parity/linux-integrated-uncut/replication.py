"""Run five predeclared slow64 pairs with the uncut ELF as the primary."""
import hashlib
import json
from pathlib import Path
import runpy
import sys
from types import ModuleType

HERE = Path(__file__).resolve().parent
TARGET = HERE.parent
ROOT = TARGET.parent
OLD = TARGET / "integrated-slow-elf-cuts"
sys.path.insert(0, str(OLD))
import canonical as frozen

SHA = frozen.SOURCE_SHA
IMAGE = frozen.SOURCE
NAMES = tuple(f"replicate-{i:02}" for i in range(1, 6))
ORDER = ("primary", "oban", "primary-repeat")
PATHS = {arm: "mesh-current" for arm in ORDER if arm != "oban"}
PINS = {"primary": SHA}
MANIFEST = HERE / "replication-freeze.json"
DRIVER = frozen.QUEUE / "canonical_current.py"


def order(name):
    return list(ORDER if NAMES.index(name) % 2 == 0 else ORDER[::-1])


def digest(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def manifest():
    frozen.verify_freeze()
    assert digest(IMAGE) == SHA
    inputs = [Path(__file__), OLD / "canonical-freeze.json", OLD / "canonical_guard.py",
              frozen.PRIOR / "shutdown_adapter.py", frozen.DRIVER / "run.py",
              frozen.DRIVER / "pack.py", frozen.DRIVER / "trial.py", DRIVER,
              frozen.QUEUE / "reduce.py"]
    return {"primary_elf_sha256": SHA, "image_path": str(IMAGE.relative_to(ROOT)),
            "primary_and_repeat_share_guest_path": PATHS,
            "groups": {name: order(name) for name in NAMES},
            "profile": "slow", "warmup_jobs": 512, "measured_jobs": 512, "hold_ms": 100,
            "slots": 64, "full_overlap_required_for_every_measured_arm": True,
            "completed_groups_are_kept_regardless_of_result": True,
            "inputs": {str(p.relative_to(ROOT)): digest(p) for p in inputs}}


def guest_source():
    route = (OLD / "route.py").read_text().split("\ndef main():", 1)[0]
    guard = (OLD / "canonical_guard.py").read_text().split("    uncut, packaged =", 1)[0]
    source = "#!/usr/bin/python3.14 -B\n" + route + "\n" + guard + """
    assert eligible(kernel, page, window)
    assert digest(PROOF / "mesh-current") == SOURCE_SHA
    print("ELF_SEGMENT_ROUTE " + json.dumps({
        "selection": "coverage", "sysname": kernel.sysname, "machine": kernel.machine,
        "release": kernel.release, "page": page, "fault_window": window,
    }), flush=True)
    os.execv("/proof/segment-canonical-init", ["/init"])


if __name__ == "__main__":
    main()
"""
    compile(source, "uncut replication guest init", "exec")
    return source.encode()


def driver():
    assert json.loads(MANIFEST.read_text()) == manifest()
    sys.path.insert(0, str(frozen.QUEUE))
    source = DRIVER.read_text()
    for before, after in (
        ('for arm in ("current-observed", "retained-normal"):', 'for arm in ():'),
        ('PINS["retained-normal"]', 'PINS["primary"]'),
        ('PINS["current-normal"]', 'PINS["primary"]'),
        ('IMAGES["current-normal"]', 'IMAGES["primary"]'),
        ('(HERE / "held-proof.json").is_file()', '(HERE / "replication-freeze.json").is_file()'),
    ):
        assert source.count(before) == 1, before
        source = source.replace(before, after)
    module = ModuleType("canonical_current")
    sys.modules[module.__name__] = module
    exec(compile(source, str(DRIVER), "exec"), module.__dict__)
    module.HERE, module.ORDER, module.ELFS = HERE, ORDER, PATHS
    module.PINS, module.IMAGES = PINS, {"primary": IMAGE}
    module.route_source = guest_source
    return module


def reduce():
    reducer = runpy.run_path(str(frozen.QUEUE / "reduce.py"))
    groups, failures = [], {}
    for name in NAMES:
        try:
            group = reducer["read_group"](HERE / name)
            assert group["order"] == order(name) and group["profile"] == "slow"
            assert not groups or group["component_pins"] == groups[0]["component_pins"]
            groups.append(group)
        except Exception as error:
            failures[name] = repr(error)
    def score(rows):
        return {arm: {mode: reducer["score"](rows, "slow", arm, shutdown)
                      for mode, shutdown in (("ordinary", False), ("through_shutdown", True))}
                for arm in PATHS}
    overlap = {g["name"]: {arm: row["peak_overlapping_committed_holds"]
                            for arm, row in g["measured"].items()} for g in groups}
    aggregate = score(groups) if groups else {}
    full = len(groups) == 5 and all(n == 64 for row in overlap.values() for n in row.values())
    frozen.write(HERE / "replication-result.json", {
        "freeze_sha256": digest(MANIFEST), "groups": groups, "incomplete": failures,
        "per_pair": {g["name"]: score([g]) for g in groups}, "aggregate": aggregate,
        "overlap": overlap, "complete": len(groups) == 5, "full_overlap": full,
        "primary_pass": full and all(not row["misses"] for row in aggregate["primary"].values()),
    })
    print("groups", len(groups), "incomplete", failures, "overlap", overlap, flush=True)
    for arm, modes in aggregate.items():
        print(arm, {mode: row["misses"] for mode, row in modes.items()}, flush=True)


def main():
    action, = sys.argv[1:]
    if action == "freeze":
        assert not any((HERE / name).exists() for name in NAMES)
        with MANIFEST.open("x") as file:
            json.dump(manifest(), file, indent=2)
            file.write("\n")
        return 0
    module = driver()
    if action == "reduce":
        reduce()
        return 0
    return module.run("slow", action, NAMES.index(action) % 2 == 1, True)


if __name__ == "__main__":
    raise SystemExit(main())
