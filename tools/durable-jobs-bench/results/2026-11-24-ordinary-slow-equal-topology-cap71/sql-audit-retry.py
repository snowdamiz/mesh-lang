"""Repair the statement-audit query and replay the same cap-71 lifecycles."""
from datetime import datetime, timezone
from pathlib import Path
import re
import sys

import packet

ID = packet.ID + "-sql-retry"
HERE = packet.PACKET
ROOT = packet.ROOT
run = packet.run
ADAPTER = HERE / "sql-audit-retry/adapter.py"
FAILED = HERE / "evidence/first-setup-failed/blocks/01-slow.1795493761849944000/warmup-mesh/warmup-mesh/raw.tar.gz"
FAILED_SHA = "43a32ab749e61ea5f2563669b77b24ce661bbd713118c6d1db420316771b2820"
ORIGINAL = {
    "setup": "bb3c4d56de7412e4a94f09f27721406c3fc566a1699f8eae23f173fbaea48ddb",
    "timing": "5c4e311bbfd5a726d8fe5efdcce10ac1185410887e3743cc27a32dc6cedbaf6b",
}
source_pins = packet.source_pins
launcher = packet.launcher
stage = run.stage


def pins():
    original = source_pins()
    for mode, digest in ORIGINAL.items():
        path = HERE / f"request-{mode}.json"
        packet.require(run.digest(path) == digest and run.read_json(path)["local_sha256"] == original,
                       f"original cap-71 {mode} inputs changed")
    packet.require(run.digest(FAILED) == FAILED_SHA, "original cap-71 failure changed")
    old = (HERE / "adapter.py").read_text()
    expected = old.replace("WHERE datname = CURRENT_DATABASE)", "WHERE datname = CURRENT_DATABASE())")
    expected = expected.replace('STATEMENTS.replace("CURRENT_DATABASE",', 'STATEMENTS.replace("CURRENT_DATABASE()",')
    packet.require(ADAPTER.read_text() == expected and expected != old, "cap-71 change exceeds the SQL audit")
    return {**original, **{str(path.relative_to(ROOT)): run.digest(path) for path in (ADAPTER, Path(__file__))}}


def checked_launcher(config, engine, remote):
    inventory = launcher(config, engine, remote)
    inventory["engines"][engine]["verified_files"][str(remote / "adapter.py")] = run.digest(ADAPTER)
    run.check_inventory(inventory)
    return inventory


def upload(source, *args, **kwargs):
    return stage(ADAPTER if source == HERE / "adapter.py" else source, *args, **kwargs)


def grant(mode, expected_pins):
    stage = mode.upper()
    request_id = f"{ID}-{mode}"
    entries = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                         (packet.QUEUE / "STATUS.md").read_text(), re.M)
    kind, key, start, end = entries[-1]
    packet.require((kind, key) == (stage, request_id)
                   and datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end),
                   f"require active cap-71 {stage} grant")
    request = HERE / f"request-sql-retry-{mode}.json"
    inbox = packet.QUEUE / f"{request_id}.json"
    packet.require(request.read_bytes() == inbox.read_bytes() and run.read_json(request)["local_sha256"] == expected_pins,
                   "cap-71 SQL retry request changed")
    return dict(start=start, end=end, request_sha256=run.digest(request), mode=stage)


if __name__ == "__main__":
    packet.launcher, packet.grant, packet.source_pins = checked_launcher, grant, pins
    run.stage = upload
    sys.exit(packet.main())
