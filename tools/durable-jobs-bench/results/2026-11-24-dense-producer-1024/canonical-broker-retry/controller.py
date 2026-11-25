"""Stage a canonical broker and require full native diagnostics before any timing."""
import argparse
from datetime import datetime, timedelta, timezone
import fcntl
import json
from pathlib import Path
import re
import runpy
import subprocess
import sys
import tarfile
from uuid import uuid4

HERE = Path(__file__).resolve().parent
PREVIOUS = HERE.parent / "timing-context-verified"
FIRST = HERE.parent / "timing-splice-retry"
sys.path.insert(0, str(PREVIOUS))
import context_retry as prior

retry = prior.retry
original, run, require, matrix = retry.original, retry.run, retry.require, retry.matrix
PACKET = HERE / "intake.json"
EVIDENCE = HERE / "evidence"
SMOKE = EVIDENCE / "smoke"
LABELS = ("128-a", original.WIDE)


def bind():
    _, packet, broker = prior.bind()
    request = run.read_json(PACKET)
    for path, digest in request["pins"].items():
        require(run.digest(retry.ROOT / path) == digest, f"canonical retry pin changed: {path}")
    preceding = PREVIOUS / "evidence/pilot"
    summary = run.read_json(preceding / "replay.json")
    release = run.read_json(preceding / "controller-release.json")
    require(release["id"] == original.ID + "-pilot-context-verified" and release["exit_status"] == 1
            and release["released_at"] and not summary["qualifies"] and summary["missing"] == [1, 2]
            and len(summary["raw"]) == 12, "preceding pilot did not release as rejected")
    failure_path = str(original.remote_stage(packet) / "packet-timing-context-verified/../broker/mesh")
    manifest = run.read_json(preceding / "series.json")
    config = original.make_config(packet, broker, "pilot")
    require(config == manifest["config"], "old scope drifted")
    rejected = {}
    for block in manifest["schedule"]:
        attempt, = (preceding / "blocks").glob(f"{run.block_prefix(block)}.*")
        for phase, label, _, folder in matrix.paths(attempt, config, block):
            path = folder / "raw.tar.gz"
            relative = str(path.relative_to(preceding))
            require(run.digest(path) == summary["raw"][relative], "prior raw archive changed")
            with tarfile.open(path) as raw:
                failure = run.file_json(raw, "adapter/failure.json")
                require(run.file_json(raw, "status.json")["exit_code"] == 1
                        and failure["message"] == f"noncanonical executable path: {failure_path}"
                        and any(frame[2] == "capture_runners" for frame in failure["stage"]),
                        "archived failure is not the capture rejection")
            try:
                original.score_arm(folder, phase, label, config, run.REMOTE / manifest["remotes"][label])
            except ValueError as error:
                require(str(error) == "arm failed or wrong invocation", "wrong archive rejection")
                rejected[relative] = summary["raw"][relative]
            else:
                raise ValueError("failed old arm received a score")
    return request, packet, broker, rejected


def sibling(config):
    return Path(config["private_stage"]) / "packet-canonical-broker"


def stage_request(mode):
    path = HERE / (mode + ".json")
    document = run.read_json(path)
    require(document["id"] == original.ID + f"-{mode}-canonical-broker"
            and document["intake_sha256"] == run.digest(PACKET), "canonical request changed")
    if mode != "smoke":
        require(document["groups"] == [1, 2], "frozen timing groups changed")
    return document, path


def grant(mode):
    request, path = stage_request(mode)
    kind, ident, start, end = retry.current_event()
    expected = "SETUP" if mode == "smoke" else "TIMING"
    require((kind, ident) == (expected, request["id"]) and end
            and datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
            and (matrix.QUEUE / (ident + ".json")).read_bytes() == path.read_bytes(),
            "matching live controller grant and sealed inbox bytes required")
    return dict(id=ident, start=start, end=end, request_sha256=run.digest(path))


def stage_verified(path, *_):
    require(path.read_text().splitlines()[-1:] == ["CANONICAL-BROKER-STAGE-VERIFIED"],
            "immutable stage readback did not finish")


def configure(request, packet, broker):
    retry.HERE, retry.PACKET, retry.EVIDENCE = HERE, PACKET, EVIDENCE
    retry.HOST_PAYLOAD = ("adapter.py", "native-run")
    retry.bind = lambda: (request, packet, broker)
    retry.sibling = sibling
    retry.stage_request = stage_request
    retry.grant = grant
    retry.check_preflight = stage_verified
    retry.use_retry("pilot")


def score_smoke(arm, label, config, remote):
    call = run.read_json(arm / "dispatch.json")
    require(call.get("exit_code") == 0, f"{label} diagnostic failed, preserve raw evidence")
    work = original.score_arm(arm, "diagnostic", label, config, remote)
    with tarfile.open(arm / "raw.tar.gz") as raw:
        def get(name):
            return run.file_json(raw, f"adapter/{name}.json")
        broker, *runners = get("process-identity")
        writer = get("producer-writer")
        ledger = get("ledger")
        producer, = ledger["transactions"]
        # The frozen scorer already checks the canonical input, ordered IDs, TLS,
        # independent application XIDs, terminal cohorts and both drained runners.
        return dict(raw_sha256=work["raw_sha256"], completed=work["completed"],
                    receipts=len(get("producer-receipts")), statements=writer["statements"],
                    producer_xid=producer["xid"], broker_argv=broker["argv"],
                    broker_sha256=broker["executable_sha256"],
                    runner_count=len(runners), runner_sha256=sorted({row["executable_sha256"] for row in runners}),
                    independent_app_transactions=len({job[k] for job in ledger["jobs"]
                                                      for k in ("attempt_xid", "effect_xid")}),
                    terminal_cohorts=get("queue-policy")["terminal_xids"])


def smoke_replay(config, attempt):
    dispatch = run.read_json(attempt / "plan.json")
    require(dispatch["config"] == config and dispatch["order"] == list(LABELS)
            and dispatch["phase"] == "diagnostic", "diagnostic binding changed")
    remote = run.REMOTE / dispatch["remote"]
    return {label: score_smoke(attempt / label / "diagnostic-mesh", label, config, remote / label)
            for label in LABELS}


def smoke(config):
    stamp = grant("smoke")
    retry.install(config, "smoke")
    attempt = SMOKE / "attempts" / uuid4().hex
    attempt.mkdir(parents=True)
    remote = run.REMOTE / f"canonical-broker-smoke-{attempt.name}"
    run.write_json(attempt / "plan.json", dict(config=config, order=list(LABELS),
                                                remote=remote.name, phase="diagnostic", grant=stamp))
    proof = {}
    with (original.dense.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        for label in LABELS:
            grant("smoke")
            lane = attempt / label
            lane.mkdir()
            files, inventory = retry.stages(config, label, remote / label)
            inv = lane / "inventory.json"
            run.write_json(inv, inventory)
            with (lane / "transfer.log").open("xb") as log:
                for path in [*files, inv]:
                    run.stage(path, remote / label, log)
            grant("smoke")
            run.dispatch_arm(lane, remote / label, {"profile": "noop"}, "diagnostic", "mesh")
            # Reject before staging or launching the next width.
            proof[label] = score_smoke(lane / "diagnostic-mesh", label, config, remote / label)
            run.write_json(lane / "correctness.json", proof[label])
    require(smoke_replay(config, attempt) == proof, "diagnostic failed offline replay")
    run.write_json(attempt / "correctness.json", proof)
    print(attempt, flush=True)


def smoke_release(config):
    request, path = stage_request("smoke")
    receipt = SMOKE / "controller-release.json"
    require(receipt.is_file(), "native diagnostic release required before timing")
    release = run.read_json(receipt)
    require(release["id"] == request["id"] and release["request_sha256"] == run.digest(path)
            and release["exit_status"] == 0 and release["released_at"],
            "native diagnostic release required before timing")
    require(re.fullmatch("[a-f0-9]{32}", release["attempt"]) is not None, "invalid smoke attempt")
    attempt = SMOKE / "attempts" / release["attempt"]
    attempt_grant = run.read_json(attempt / "plan.json")["grant"]
    require(attempt_grant["id"] == release["id"]
            and attempt_grant["request_sha256"] == release["request_sha256"],
            "native diagnostic attempt belongs to another grant")
    result = smoke_replay(config, attempt)
    require(result == run.read_json(attempt / "correctness.json"), "diagnostic receipt changed")
    return result


def smoke_slot(config, digest):
    request, source = stage_request("smoke")
    ident = request["id"]
    require(digest == run.digest(source), "sealed smoke request SHA-256 required")
    require((matrix.QUEUE / (ident + ".json")).read_bytes() == source.read_bytes(), "controller inbox changed")
    SMOKE.mkdir(parents=True, exist_ok=True)
    with (SMOKE / ".slot.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        require(retry.current_event()[0] == "RELEASED" and not (SMOKE / "controller-release.json").exists(),
                "controller busy or smoke already released")
        ops = runpy.run_path(str(matrix.QUEUE.parent / "completion-writer-r2.py"))
        ops["probe"]()
        start = datetime.now(timezone.utc)
        end = start + timedelta(minutes=request["requested_window_minutes"])
        ops["note"](f"SETUP {ident} {start:%Y-%m-%dT%H:%M:%SZ} {end:%Y-%m-%dT%H:%M:%SZ}\n\n"
                    f"Two fail-fast diagnostic arms. Correctness only. Seal {digest}.")
        report = dict(id=ident, request_sha256=digest, started=start.isoformat(), exit_status=1)
        try:
            with (SMOKE / "controller.log").open("xb") as log:
                child = subprocess.run([sys.executable, "-B", str(HERE / "controller.py"), "dispatch", "smoke"],
                                       cwd=retry.ROOT, stdout=log, stderr=log)
            if child.returncode == 0:
                complete = [p for p in (SMOKE / "attempts").iterdir() if (p / "correctness.json").is_file()]
                require(len(complete) == 1, "expected one reconciled native diagnostic attempt")
                attempt, = complete
                require(smoke_replay(config, attempt) == run.read_json(attempt / "correctness.json"),
                        "cannot release native diagnostic")
                report.update(exit_status=0, attempt=attempt.name)
        finally:
            try:
                ops["probe"]()
            except (OSError, subprocess.SubprocessError) as error:
                run.write_json(SMOKE / "release-pending.json", dict(report, lock_probe_error=str(error)))
                raise
            report["released_at"] = run.utc_now()
            run.write_json(SMOKE / "controller-release.json", report)
            run.write_json(matrix.QUEUE / f"{ident}.released.json", report)
            ops["note"](f"RELEASED {ident} {report['released_at']}, exit {report['exit_status']}. Evidence: {SMOKE}")
        return report["exit_status"]


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("command", choices=("check", "dispatch", "replay", "slot"))
    cli.add_argument("mode", nargs="?", choices=("smoke", "pilot", "peers"))
    cli.add_argument("sha", nargs="?")
    args = cli.parse_args()
    request, packet, broker, rejected = bind()
    configure(request, packet, broker)
    config = original.make_config(packet, broker, "pilot")
    if args.command == "check":
        for mode in ("smoke", "pilot", "peers"):
            stage_request(mode)
        for mode in ("pilot", "peers"):
            retry.check_stages(original.make_config(packet, broker, mode))
        from score_controls import run_controls
        from identity_controls import check_local
        print(json.dumps(dict(rejected=rejected, identity=check_local(EVIDENCE / "local-identity.log"),
                              score_controls=run_controls(original, config, retry.stages,
                                                          retry.ROOT / request["setup_raw"])),
                         indent=2, sort_keys=True))
        return 0
    require(args.mode, "specify smoke, pilot or peers")
    if args.mode == "smoke":
        if args.command == "replay":
            print(json.dumps(smoke_release(config), indent=2, sort_keys=True))
            return 0
        if args.command == "slot":
            return smoke_slot(config, args.sha)
        smoke(config)
        return 0
    # Replay and dispatch both retain the diagnostic certificate and selection.
    smoke_release(config)
    sys.argv = [sys.argv[0], args.command, args.mode] + ([args.sha] if args.sha else [])
    return retry.entry()


if __name__ == "__main__":
    sys.exit(main())
