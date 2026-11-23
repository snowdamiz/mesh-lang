"""Qualify both exact existing V2 slow images under the proof reservation."""

from datetime import datetime, timezone
import fcntl
from pathlib import Path
import re
import sys
import tarfile
from uuid import uuid4

from series import HERE, V2, matrix, run, validate


ID = "bulk-enqueue-v2-slow-qualification"


def require_grant():
    status = (HERE.parent / "requests/STATUS.md").read_text()
    kind, name, start, end = re.findall(r"^(SETUP|TIMING|RELEASED) (\S+) (\S+)(?: (\S+))?", status, re.M)[-1]
    now = datetime.now(timezone.utc)
    run.require((kind, name) == ("SETUP", ID) and datetime.fromisoformat(start) <= now < datetime.fromisoformat(end),
                "an active exclusive V2 slow proof reservation is required")


def qualify(config, output):
    block = {"profile": "slow", "order": sorted(V2), "phases": ["diagnostic"]}
    with (run.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        require_grant()
        run.ssh(["bash", "-c", matrix.HOST_IDLE], timeout=12).check_returncode()
        output.mkdir()
        run.write_json(output / "order.json", block)
        run.write_json(output / "inventory.json", config)
        nonce = uuid4().hex[:12]
        for label in block["order"]:
            require_grant()
            local = output / f"diagnostic-{label}"
            local.mkdir()
            inventory = local / "inventory.json"
            run.write_json(inventory, config["lanes"][label]["inventory"])
            remote = run.REMOTE / f"{ID}-{nonce}-{label}"
            with (local / "stage.log").open("xb") as log:
                run.stage(run.SUITE / "worker.py", remote, log)
                run.stage(inventory, remote, log)
            run.dispatch_arm(local, remote, block, "diagnostic", "mesh")
        result = matrix.replay(output, config, block)
        for _, label, folder in matrix.arms(output, config, block):
            with tarfile.open(folder / "raw.tar.gz", "r:gz") as data:
                ledger = run.file_json(data, "adapter/ledger.json")
                settings = run.file_json(data, "adapter/postgres-settings.json")
                writer = run.file_json(data, "adapter/producer-writer.json")
            run.require(settings["max_connections"] == settings["database_connection_limit"] == 68
                        and settings["version"] == "17.11" and settings["tls"] is True,
                        "slow database cap, version or TLS differs from the frozen profile")
            jobs = ledger["jobs"]
            run.require(len(jobs) == 512 and ledger["slots"] == 64,
                        "V2 slow workload or configured occupancy changed")
            result[label] = {"jobs": len(jobs), "terminal_xids": len({row["settled_xid"] for row in jobs}),
                             "writer": writer, "postgres": settings, "peak_connections": ledger["peak_connections"],
                             "peak_executing": ledger["peak_executing"]}
        run.ssh(["bash", "-c", matrix.HOST_IDLE], timeout=12).check_returncode()
        run.write_json(output / "qualified.json", result)


if __name__ == "__main__":
    config = run.read_json(Path(sys.argv[1]))
    validate(config)
    if sys.argv[2:] == ["--check"]:
        print("V2 slow seven-lane inventory validated offline")
    else:
        qualify(config, Path(sys.argv[2]))
