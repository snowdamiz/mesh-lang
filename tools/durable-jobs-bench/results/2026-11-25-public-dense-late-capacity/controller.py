"""Controller-only entry points for one proof slot and its dependent pilot."""
import argparse
from datetime import datetime, timezone
import fcntl
from pathlib import Path
import runpy
import sys

import qualification

HERE = Path(__file__).resolve().parent


def load_controller(controller):
    global pilot, HOST_IDLE
    controller_root = controller.parent.parent
    suite = controller_root / "tools/durable-jobs-bench"
    sys.path.insert(0, str(suite))
    import run as controller_run
    from interleave import HOST_IDLE
    import pilot
    assert controller_run.SUITE == suite and pilot.run is controller_run
    assert controller_run.ACCESS == controller_root / "target/linode-jobs-access"
    pilot.verified_dependencies()
    for name in pilot.FILES:
        assert controller_run.digest(suite / name) == controller_run.digest(pilot.SUITE / name)
    return controller_run


def registered(queue, identity, path):
    assert (queue / f"{identity}.json").read_bytes() == path.read_bytes()
    return pilot.run.digest(path)


def setup(controller, source, bundle):
    bridge = runpy.run_path(str(controller / "public-fixed8-product-setup.py"))
    scope = bridge["execute"].__globals__
    packet = qualification.check(qualification.driver(source), bundle)
    seal = registered(scope["QUEUE"], packet["id"], bundle / "jobs.json")
    assert seal == pilot.run.read_json(HERE / "setup-seal.json")["request_sha256"]
    output = bundle / "evidence"
    assert output.resolve() == Path(packet["evidence_destination"])

    def replay(release, raw, evidence, label):
        pilot.run.write_json(evidence / label, qualification.replay(source, bundle, release, raw))
        return 0

    scope.update(ID=packet["id"], SCRIPT=bundle / "native.sh", SOURCE=bundle / "source.tar.gz", replay=replay)
    receipt = scope["FENCE"]["slot"](
        "SETUP", packet["id"], seal, packet["requested_window_minutes"], output,
        lambda controls, grant: scope["execute"](packet, output, controls, grant),
    )
    if receipt["exit_status"] == 0 and not (output / "verified.json").exists():
        replay(output / "controller-release.json", output / "raw.tar.gz", output, "verified.json")
    return receipt["exit_status"]


def timing(controller, source, bundle, bound):
    state = runpy.run_path(str(controller / "remaining-booking-fence.py"))
    packet, qualified = pilot.checked_binding(bound)
    proof = bundle / "evidence"
    setup_release = proof / "controller-release.json"
    same = qualification.replay(source, bundle, setup_release, proof / "raw.tar.gz")
    assert qualified == same and packet["setup_release_sha256"] == pilot.run.digest(setup_release)
    assert (state["QUEUE"] / f"{qualified['packet']}.released.json").read_bytes() == setup_release.read_bytes()
    identity = packet["timing_id"]
    seal = registered(state["QUEUE"], identity, bound / "controller-packet.json")
    runner = state["RUN"]
    output = bound / "evidence"

    def execute(_controls, grant):
        def active(minimum_seconds):
            assert registered(state["QUEUE"], identity, bound / "controller-packet.json") == seal
            assert state["latest"]()[-1] == ("TIMING", identity)
            now = datetime.now(timezone.utc)
            assert datetime.fromisoformat(grant["start"]) <= now
            assert (datetime.fromisoformat(grant["end"]) - now).total_seconds() >= minimum_seconds

        def dispatch(folder, label, order, phase):
            active(125)
            folder.parent.mkdir()
            runner.dispatch_arm(folder.parent, pilot.remote(label), order, phase, "mesh")

        with (runner.SUITE / ".timing.lock").open("rb") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            active(125)
            runner.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
            output.mkdir(parents=True, exist_ok=False)
            runner.write_json(output / "grant.json", grant)
            for label in ("A1", "reference", "B"):
                log_dir = output / "staging" / label
                log_dir.mkdir(parents=True)
                staging = bound / "staging" / label
                with (log_dir / "stage.log").open("xb") as log:
                    for item in (*sorted(staging.iterdir()), pilot.SUITE / "worker.py"):
                        active(125)
                        runner.stage(item, pilot.remote(label), log)
                (log_dir / "inventory.json").write_bytes((staging / "inventory.json").read_bytes())
            result = pilot.visit(output, bound, dispatch)
            runner.write_json(output / "scored-before-release.json", result)
            active(3)
            runner.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
        return 0

    return state["slot"]("TIMING", identity, seal, packet["minutes"], output, execute)["exit_status"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=("setup", "timing"))
    parser.add_argument("controller", type=Path)
    parser.add_argument("source", type=Path)
    parser.add_argument("bundle", type=Path)
    parser.add_argument("bound", type=Path, nargs="?")
    args = parser.parse_args()
    controller, source, bundle = (p.resolve(strict=True) for p in (args.controller, args.source, args.bundle))
    assert controller.parent.parent != HERE.parents[3]
    load_controller(controller)
    with (controller / f"{pilot.ID}.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return (setup(controller, source, bundle) if args.phase == "setup"
                else timing(controller, source, bundle, args.bound.resolve(strict=True)))


if __name__ == "__main__":
    sys.exit(main())
