"""Run the original six-arm screen in the sole controller's fenced slot."""
from datetime import datetime, timezone
import fcntl
from pathlib import Path
import runpy
import shutil
import sys
from uuid import uuid4

import retry

run, out, require = retry.run, retry.OUT, retry.require
STATE = retry.ROOT / "target/linode-jobs-stepchange"
SEAL = retry.HERE / "controller-packet.json"


def main(seal):
    packet = retry.checked()
    require(run.digest(SEAL) == seal, "sibling seal differs")
    fence = runpy.run_path(str(STATE / "remaining-booking-fence.py"))
    queue = fence["QUEUE"]
    inbox = queue / f"{retry.ID}.json"
    require(inbox.read_bytes() == SEAL.read_bytes(), "sibling is not registered")
    group = packet["group"]
    stages = {lane: str(run.REMOTE / f"postflush-{uuid4().hex[:12]}-{lane}") for lane in group["order"]}

    def execute(controls, grant):
        def left(needed=0):
            now = datetime.now(timezone.utc)
            start, end = (datetime.fromisoformat(grant[key]) for key in ("start", "end"))
            require(fence["latest"]()[-1] == ("TIMING", retry.ID) and start <= now < end
                    and inbox.read_bytes() == SEAL.read_bytes(), "inactive or changed grant")
            remaining = int((end - now).total_seconds()) - packet["release_reserve_seconds"]
            require(remaining > needed, "insufficient time before release")
            return remaining

        out.mkdir()
        run.write_json(out / "grant.json", grant)
        run.write_json(out / "stages.json", stages)
        (out / "preflight").mkdir()
        with (run.SUITE / ".timing.lock").open("rb") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for lane, stage in stages.items():
                folder = out / "staging" / lane
                folder.mkdir(parents=True)
                pinned = retry.inventory(stage, lane)
                run.write_json(folder / "inventory.json", pinned)
                with (folder / "stage.log").open("xb") as log:
                    for name, relative in packet["arm_files"].items():
                        path = retry.ROOT / relative
                        require(run.digest(path) == packet["local_sha256"][relative], "stage input changed")
                        if name != path.name:
                            path = Path(shutil.copyfile(path, folder / name))
                        left(30)
                        run.stage(path, Path(stage), log)
                    left(30)
                    run.stage(folder / "inventory.json", Path(stage), log)
                preflight = out / "preflight" / f"{lane}.json"
                with preflight.open("xb") as proof, preflight.with_suffix(".log").open("xb") as log:
                    result = run.ssh(["bash", Path(stage) / "native-preflight.sh", lane],
                                     stdout=proof, stderr=log, timeout=min(45, left(45)))
                result.check_returncode()
                retry.verify_activation(run.read_json(preflight), lane)
            block = out / run.block_prefix(group)
            block.mkdir()
            run.write_json(block / "order.json", group)
            for phase in group["phases"]:
                for lane in group["order"]:
                    left(packet["arm_timeout_seconds"] + 5)
                    arm = block / f"{phase}-{lane}"
                    arm.mkdir()
                    run.dispatch_arm(arm, Path(stages[lane]), group, phase, "mesh")
                    retry.score.score_arm(arm / f"{phase}-mesh", phase,
                                          retry.inventory(stages[lane], lane), lane == "on")
        run.write_json(out / "summary.json", retry.replay(out))
        return 0

    with (STATE / f"{retry.ID}.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        receipt = fence["slot"]("TIMING", retry.ID, seal, packet["minutes"], out, execute)
    return receipt["exit_status"]


if __name__ == "__main__":
    sys.exit(main(sys.argv[1]))
