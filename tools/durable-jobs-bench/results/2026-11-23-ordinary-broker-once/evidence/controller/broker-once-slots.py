"""Use the emitted broker-once SETUP evidence in each sealed timing command."""

from pathlib import Path
import runpy
import subprocess
import sys

STATE = Path(__file__).resolve().parent
registry = runpy.run_path(str(STATE / "hot-checkout-slots.py"))
run, timing = registry["run"], registry["timing"]
ID = "ordinary-broker-once-setup"
release_file = STATE / "requests" / f"{ID}.released.json"
if not release_file.is_file():
    subprocess.run([sys.executable, "-B", str(STATE / "ready-command.py"), ID], check=True)
release = run.read_json(release_file)
run.require(release["exit_status"] == 0, "the broker-once proof did not qualify")
controller = STATE / "controller-receipts" / ID
request = run.read_json(controller / "request.json")
root = Path(request["evidence_destination"]).resolve()
directories = [candidate for line in (controller / "controller.log").read_text().splitlines()
               if (candidate := Path(line.strip())).is_absolute() and candidate.parent == root
               and candidate.name.startswith(f"{ID}-") and (candidate / "setup-status.json").is_file()]
run.require(len(directories) == 1, "expected one printed setup evidence directory")
qualified = directories[0]
status = run.read_json(qualified / "setup-status.json")
run.require(status["freeze"] == request["freeze_sha256"] and len(status["qualified"]) == 6
            and all(status["qualified"].values()), "broker-once six-lane proof differs from the sealed freeze")
followup = request["conditional_follow_up"]
request["timing"] = {"id": followup["id"],
                     "exact_group_commands": [cmd.replace("<SETUP_EVIDENCE>", str(qualified)) for cmd in followup["commands"]],
                     "summarize_command": followup["summarize_command"].replace("<SETUP_EVIDENCE>", str(qualified))}
sys.exit(timing(request, [1, 2, 3, 4, 5]))
