"""Run the signed dense experiment with a separate journal-parser recovery grant."""
from datetime import datetime, timezone
import json
from pathlib import Path
import runpy
import sys

HERE = Path(__file__).resolve().parent
module = runpy.run_path(str(HERE / "amendment.py"))
original = module["ID"]
identifier = original + "-grant-recovery"
require = module["require"]
queue = module["dense"].QUEUE
packet = HERE / "grant-recovery.json"


def active_window(journal, now):
    events = [line.split() for line in journal.splitlines()
              if line.split()[:1] in (["SETUP"], ["TIMING"], ["RELEASED"], ["HOLD"])]
    require(bool(events), "controller journal contains no grant")
    event = events[-1]
    require(len(event) == 4 and event[:2] == ["TIMING", identifier], "dedicated TIMING grant required")
    start, end = event[2:]
    require(datetime.fromisoformat(start) <= now < datetime.fromisoformat(end), "inactive grant interval")
    return start, end


def grant(_=None):
    start, end = active_window((queue / "STATUS.md").read_text(), datetime.now(timezone.utc))
    for source, key in ((packet, identifier), (module["REQUEST"], original),
                        (module["dense"].REQUEST, module["dense"].ID)):
        require(source.read_bytes() == (queue / f"{key}.json").read_bytes(), f"inbox seal changed: {key}")
    return dict(start=start, end=end, request_sha256=module["run"].digest(module["REQUEST"]),
                journal_recovery_sha256=module["run"].digest(packet))


if __name__ == "__main__":
    config = json.loads(packet.read_bytes())
    for name, expected in config["files"].items():
        require(module["run"].digest(HERE / name) == expected, f"recovery file changed: {name}")
    module["main"].__globals__["grant"] = grant
    sys.exit(module["main"]())
