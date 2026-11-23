#!/usr/bin/env python3
"""Verify the controller's archived strict/cohort smoke ledgers without SSH."""
import argparse
import json
from pathlib import Path
import sys
import tarfile

sys.path.insert(0, str(Path.cwd() / "tools/durable-jobs-bench"))
from verify import reconcile

MODES = {"strict-noop", "strict-short", "cohort-noop", "cohort-short"}


def reduce(archive, revision):
    with tarfile.open(archive, "r:gz") as export:
        suffix = "/admission-ledger.json"
        roots = {entry.name.removesuffix(suffix) for entry in export if entry.name.endswith(suffix)}
        scores = {}
        for root in sorted(roots):
            name = Path(root).name
            assert name in MODES
            mode, profile = name.split("-")
            read = lambda file: json.load(export.extractfile(f"{root}/{file}"))
            invocation = read("invocation.json")
            assert invocation["mesh_admission"] == mode
            build, ledger, proof = (read(file) for file in ("build.json", "ledger.json", "admission-ledger.json"))
            assert build["version"] == ledger["version"] == revision
            score = reconcile(ledger, "mesh", profile, {"version": revision, "phase": "diagnostic"})
            n = score["completed"]
            jobs = ledger["jobs"]
            assert len({job["worker_pid"] for job in jobs}) == n
            terminal = len({job["settled_xid"] for job in jobs})
            assert terminal == proof["terminal_xids"] and proof["mode"] == mode
            assert proof["admission_turns"] > 0
            if mode == "strict":
                assert terminal == n and proof["cohort_turns"] == 0
            assert proof["resets"] == 2 * n + (32 if profile == "noop" else 8)
            host = read("host-before.json")
            affinity = read("process-affinity.json")
            ordered = read("ordered-writes.json")
            assert host["architecture"] == "x86_64" and host["cpus"] == [0, 1]
            assert affinity and all(mask == [0, 1] for mask in affinity.values())
            assert ordered["jobs"] == ordered["ordered"] == ordered["worker_pids"] == n
            scores[name] = dict(score=score, dispatch_proof=proof, executable_sha256=build["files"]["mesh"],
                                artifact_sha256=ledger["artifact_sha256"], affinity=[0, 1],
                                database=ledger["database"])
        assert scores.keys() == MODES, scores.keys()
    return scores


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("archive", type=Path)
    cli.add_argument("revision")
    a = cli.parse_args()
    print(json.dumps(reduce(a.archive, a.revision), indent=2))
