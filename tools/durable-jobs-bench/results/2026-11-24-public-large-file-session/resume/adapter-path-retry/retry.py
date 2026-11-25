"""Retry the native timing matrix against the verified installed adapter path."""

from datetime import datetime, timedelta, timezone
import json
from pathlib import Path
import re
import runpy
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import controller as original
import slot

run, parked = original.run, original.parked
NATIVE = HERE.parent / "evidence/native"
RECOVERED = NATIVE / "setup-export-recovered"
OUT = NATIVE / "timing-adapter-path-retry"
ID = original.ID + "-adapter-path-retry"
INBOX = parked.QUEUE / f"{ID}.json"
PACKET = HERE / "intake.json"
BAD = "replay-public-large-file-session"
GOOD = "replay-public-large-file-session-resume"
recovered = runpy.run_path(str(HERE.parent / "recovered-timing.py"))
original_plan = original.timing_plan


def document():
    doc = run.read_json(PACKET)
    assert (doc["id"], doc["ready"]) == (ID, True)
    for path, digest in doc["local_sha256"].items():
        assert original.sha(parked.ROOT / path) == digest, path
    return recovered["recovered_document"]()


def plan(doc, setup_dir):
    bound = original_plan(doc, setup_dir)
    assert bound["session_qualification"]["runtime_origin"] == "rebuilt-exact"
    assert setup_dir == RECOVERED
    with tarfile.open(setup_dir / "raw.tar.gz") as archive:
        installed = {path: digest for digest, path in
                     (line.split() for line in archive.extractfile("./adapter.sha256").read().decode().splitlines())}
    lanes = bound["configuration"]["lanes"]
    for label in parked.MESH[1:]:
        engine = lanes[label]["inventory"]["engines"]["mesh"]
        registered = engine["verified_files"]
        wrong = {path: digest for path, digest in registered.items() if Path(path).parent.name == BAD}
        assert {Path(path).name: digest for path, digest in wrong.items()} == doc["adapter_sha256"]
        right = {str(Path(path).parent.with_name(GOOD) / Path(path).name): digest
                 for path, digest in wrong.items()}
        assert right == installed
        engine["verified_files"] = {path: digest for path, digest in registered.items() if path not in wrong}
        engine["verified_files"].update(right)
        (index,) = [i for i, path in enumerate(engine["argv"]) if Path(path).parent.name == BAD]
        engine["argv"][index] = str(Path(engine["argv"][index]).parent.with_name(GOOD) / "native-run")
    original.validate(bound["configuration"])
    return bound


def check(doc):
    packet = run.read_json(PACKET)
    initial = original_plan(doc, RECOVERED)
    revised = plan(doc, RECOVERED)
    assert revised["recipe"] == initial["recipe"]
    before = initial["configuration"]["lanes"]
    after = revised["configuration"]["lanes"]
    assert all(before[label] == after[label] for label in ("product", "river", "oban", "apalis"))
    assert before["inline-a"] == before["inline-b"] and after["inline-a"] == after["inline-b"]
    for label, name in packet["failed_receipts"].items():
        with tarfile.open(parked.ROOT / name) as archive:
            assert set(archive.getnames()) == {"error.json", "invocation.json", "status.json"}
            error = run.file_json(archive, "error.json")
            assert error["type"] == "FileNotFoundError"
            assert f"/{BAD}/backend.py" in error["error"], label
    print("Recovered SETUP, three path failures, corrected installed pins, and unchanged seven-lane plan verified")


def last_grant():
    pattern = r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?"
    return re.findall(pattern, (parked.QUEUE / "STATUS.md").read_text(), re.M)[-1]


def assignment(mode):
    kind, key, start, end = last_grant()
    assert (kind, key, mode) == ("TIMING", ID, "TIMING")
    assert datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
    assert INBOX.read_bytes() == PACKET.read_bytes()
    return dict(kind=kind, key=key, start=start, end=end)


def open_timing(doc):
    check(doc)
    assert INBOX.read_bytes() == PACKET.read_bytes() and not OUT.exists()
    original_release = run.read_json(parked.QUEUE / f"{original.ID}.timing.released.json")
    assert (original_release["id"], original_release["phase"]) == (original.ID, "timing")
    assert original_release["exit_status"] != 0
    assert last_grant()[0] == "RELEASED"
    slot.probe()
    start = datetime.now(timezone.utc)
    end = start + timedelta(minutes=24)
    slot.note(f"TIMING {ID} {start:%Y-%m-%dT%H:%M:%SZ} {end:%Y-%m-%dT%H:%M:%SZ}\n\n"
              f"Owner: matrix implementation operator. Adapter path correction. Evidence: {OUT}.")
    grant = assignment("TIMING")
    log = NATIVE / "timing-adapter-path-retry.log"
    result = 1
    try:
        with log.open("xb") as stream:
            result = subprocess.run([sys.executable, "-B", __file__, "dispatch"], cwd=parked.ROOT,
                                    stdout=stream, stderr=subprocess.STDOUT).returncode
    finally:
        receipt = dict(id=ID, phase="timing", grant=grant, exit_status=result, receipt=str(OUT),
                       controller_log=str(log), command_finished_at=run.utc_now())
        try:
            slot.probe()
        except (OSError, subprocess.SubprocessError) as error:
            receipt["release_probe_error"] = str(error)
            run.write_json(NATIVE / "timing-adapter-path-retry-release-pending.json", receipt)
            slot.note(f"RELEASE PENDING {ID} {run.utc_now()}, command exit {result}. Lock probe failed.")
            raise
        receipt["released_at"] = run.utc_now()
        run.write_json(parked.QUEUE / f"{ID}.timing.released.json", receipt)
        run.write_json(NATIVE / "timing-adapter-path-retry-release.json", receipt)
        slot.note(f"RELEASED {ID} {receipt['released_at']}, exit {result}. Evidence: {OUT}")
    return result


if __name__ == "__main__":
    action, = sys.argv[1:]
    doc = document()
    if action == "check":
        check(doc)
    elif action == "timing":
        sys.exit(open_timing(doc))
    elif action in ("dispatch", "scores"):
        original.timing_plan = plan
        if action == "dispatch":
            original.assignment = assignment
            sys.exit(original.timing(doc, RECOVERED, OUT))
        release = run.read_json(NATIVE / "timing-adapter-path-retry-release.json")
        assert (release["id"], release["exit_status"]) == (ID, 0)
        print(json.dumps(original.scores(doc, RECOVERED, OUT), sort_keys=True, indent=2))
    else:
        sys.exit("expected check, timing, dispatch or scores")
