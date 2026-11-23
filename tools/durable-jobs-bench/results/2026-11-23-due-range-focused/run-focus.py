"""Run five alternating old/new Mesh no-op pairs on the sealed native host."""

import fcntl
from pathlib import Path
from statistics import median
import sys
import tarfile
import time
from uuid import uuid4

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[1]))
import run


FIELDS = ("submit_jobs_per_s", "commit_jobs_per_s", "runner_cpu_ms_per_job",
          "commit_to_pickup_p99_ms", "raw_runner_sql_calls_per_job")


def execute_pair(out, pair, variants, expected_host):
    group = out / f"pair-{pair:02d}"
    group.mkdir()
    order = list(variants) if pair % 2 else list(reversed(variants))
    block = {"profile": "noop", "order": order, "pair": pair}
    run.write_json(group / "order.json", block)
    result = {}
    for phase in ("warmup", "measured"):
        for name in order:
            config, remote = variants[name]
            folder = group / f"{phase}-{name}"
            folder.mkdir()
            run.dispatch_arm(folder, remote, block, phase, "mesh")
            arm = folder / f"{phase}-mesh"
            checked = run.score_arm(arm, block, phase, "mesh", config)
            observed_host = [checked["host_boot_id"], checked["host_kernel"], checked["pg_mount"]["maj:min"]]
            run.require(observed_host == expected_host, "focus changed the host, kernel, or disk")
            with tarfile.open(arm / "raw.tar.gz") as archive:
                raw = run.file_json(archive, "adapter/raw.json")
            checked["raw_runner_sql_calls_per_job"] = raw["raw_runner_sql_calls"] / checked["completed"]
            run.write_json(folder / "reconciled.json", checked)
            if phase == "measured":
                result[name] = checked
    run.write_json(group / "reconciled.json", result)
    return result


def main():
    candidate = run.read_json(run.RESULTS / "2026-11-23-mesh-due-range" / "inventory.json")
    original = run.read_json(run.RESULTS / "2026-11-23-baseline" / "inventory.json")
    reference = run.verify_reference("2026-11-23-baseline", candidate)
    out = HERE / f"trial-{time.time_ns()}"
    out.mkdir()
    run.write_json(out / "reference.json", reference)
    variants = {}
    with (run.SUITE / ".timing.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        for name, config in (("baseline", original), ("candidate", candidate)):
            folder = out / name
            folder.mkdir()
            remote = run.REMOTE / ("due-range-focus-" + name + "-" + uuid4().hex)
            run.write_json(folder / "inventory.json", config)
            run.write_json(folder / "host.json", {"host_series": remote.name})
            with (folder / "stage.txt").open("xb") as log:
                run.stage(run.SUITE / "worker.py", remote, log)
                run.stage(folder / "inventory.json", remote, log)
            variants[name] = config, remote
        pairs = [execute_pair(out, pair, variants, reference["host"]) for pair in range(1, 6)]
    summary = {}
    for field in FIELDS:
        before = [pair["baseline"][field] for pair in pairs]
        after = [pair["candidate"][field] for pair in pairs]
        changes = [100 * (new / old - 1) for old, new in zip(before, after, strict=True)]
        summary[field] = {"baseline": before, "candidate": after, "median_change_percent": median(changes),
                          "min_change_percent": min(changes), "max_change_percent": max(changes)}
    run.write_json(out / "summary.json", {"pairs": len(pairs), "metrics": summary})
    print(out / "summary.json")


if __name__ == "__main__":
    main()
