"""Run the sealed comparison under a separate, correctly parsed controller grant."""

from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import matched

ID = f"{HERE.parent.name}-{HERE.name}"
PACKET = HERE / "controller-packet.json"
OUT = HERE / "evidence/native"
QUEUE = matched.QUEUE
STATE = re.compile(r"^(SETUP|TIMING|RELEASED|HOLD)\b[^\n]*", re.M)
TIMING = re.compile(r"TIMING (\S+) (\S+) (\S+)")


def grant():
    inbox = QUEUE / f"{ID}.json"
    matched.run.require(inbox.is_file() and inbox.read_bytes() == PACKET.read_bytes(),
                        "registered request differs from the retry packet")
    # Find the last state record before deciding whether it grants timing.
    records = list(STATE.finditer((QUEUE / "STATUS.md").read_text()))
    active = TIMING.fullmatch(records[-1].group()) if records else None
    matched.run.require(active is not None and active[1] == ID,
                        "requires an active sole-controller TIMING grant")
    start, end = active[2], active[3]
    left, right = datetime.fromisoformat(start), datetime.fromisoformat(end)
    matched.run.require(left.tzinfo is not None and right.tzinfo is not None
                        and left <= datetime.now(timezone.utc) < right,
                        "outside the active TIMING interval")
    return dict(id=ID, start=start, end=end, request_sha256=matched.sha(PACKET))


def configuration_digest(plan):
    wire = json.dumps(plan, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()
    return hashlib.sha256(wire).hexdigest()


def check():
    parent = matched.packet()
    doc = matched.read(PACKET)
    original_path = matched.PACKET
    assert doc["id"] == ID and doc["ready"] is True
    assert doc["original_packet_sha256"] == matched.sha(original_path)
    assert doc["timing_minutes"] == parent["timing_minutes"]
    for filename, expected in doc["local_sha256"].items():
        assert matched.sha(matched.parked.ROOT / filename) == expected, filename
    doc_dir = original_path.parent / "evidence/native"
    release = matched.read(doc_dir / "controller-release.json")
    assert (release["id"], release["exit_status"], release["request_sha256"]) == (
        parent["id"], 1, doc["original_packet_sha256"])
    assert "too many values to unpack (expected 4, got 5)" in (doc_dir / "controller.log").read_text()
    plan = matched.configure()
    assert plan["recipe"]["blocks"] == parent["blocks"]
    assert configuration_digest(plan) == doc["configuration_sha256"]
    return plan


def main():
    action, = sys.argv[1:]
    plan = check()
    if action == "check":
        print(f"Verified original immutable request, failure, configurations and scoring for {ID}")
        return
    # Bind only the retry's grant and output identity. Use the original
    # matrix, launch inventories, frozen schedule and receipt reducer.
    matched.grant = grant
    matched.ID = ID
    matched.PACKET = PACKET
    matched.OUT = OUT
    if action == "timing":
        sys.exit(matched.timing(plan))
    if action == "replay":
        print(json.dumps(matched.report(plan), sort_keys=True, indent=2, allow_nan=False))
        return
    sys.exit("expected check, timing or replay")


if __name__ == "__main__":
    main()
