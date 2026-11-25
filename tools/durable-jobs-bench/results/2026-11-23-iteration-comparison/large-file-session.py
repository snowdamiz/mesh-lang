"""Replay the released public LARGE path-only retry from its native archives."""

from datetime import datetime
from pathlib import Path
import json
import runpy
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
SUITE = HERE.parents[1]
sys.path.insert(0, str(SUITE))
import run

CAMPAIGN = "2026-11-24-public-large-file-session"
RETRY_ID = CAMPAIGN + "-resume-adapter-path-retry"
ORIGINAL_INTAKE_SHA256 = "dd733073b110a4e5bc2b9041f88e25311ea224f03e95f6f70aa3bbc3fde41a76"
INTAKE_SHA256 = "6efbc42ad945f0bfaeccb4550bbe462d98ab8a488d0c5eecbcd193da4ec76fcd"
LAUNCHER_SHA256 = "96ae946a1dbb8f632a87c40309385a6187b91d8b4941c26dea4cb410b3eaa944"
FROZEN = Path("target/2026-11-24-trusted-jobs-product-composition/source")
LANES = {"file": "mesh", "inline-a": "mesh", "inline-b": "mesh", "product": "mesh",
         "river": "go", "oban": "oban", "apalis": "rust"}


def release_receipt(resume, previous_release):
    native = resume / "evidence/native"
    timing = native / "timing-adapter-path-retry"
    archive = native / "timing-adapter-path-retry-release.json"
    live = ROOT / "target/linode-jobs-stepchange/requests" / f"{RETRY_ID}.timing.released.json"
    count = len(list((timing / "blocks").glob("*/reconciled.json")))
    if not archive.is_file():
        raise ValueError(f"{count}/5 reconciled retry groups and no archived successful TIMING release")
    digest = run.digest(archive)
    if live.exists():
        run.require(live.read_bytes() == archive.read_bytes(), "live and archived TIMING releases differ")
        packet = resume / "adapter-path-retry/intake.json"
        run.require(live.with_name(f"{RETRY_ID}.json").read_bytes() == packet.read_bytes(),
                    "live and sealed retry intakes differ")
    else:
        run.require(previous_release == digest, "live TIMING release missing for first publication")
    release = run.read_json(archive)
    grant = release["grant"]
    run.require((release["id"], release["phase"], release["exit_status"], grant["key"], grant["kind"])
                == (RETRY_ID, "timing", 0, RETRY_ID, "TIMING"), "TIMING did not release successfully")
    location = Path(release["receipt"])
    run.require(location.is_absolute() and location.parts[-len(timing.relative_to(ROOT).parts):]
                == timing.relative_to(ROOT).parts, "TIMING release points at another capture")
    start, end = [datetime.fromisoformat(grant[key]) for key in ("start", "end")]
    run.require(start <= datetime.fromisoformat(release["command_finished_at"])
                <= datetime.fromisoformat(release["released_at"]) and start < end, "invalid TIMING release order")
    return timing, archive, digest, live.exists()


def sealed_source(relative):
    path = ROOT / relative
    if not path.exists() and path.is_relative_to(ROOT / FROZEN):
        return ROOT / path.relative_to(ROOT / FROZEN)
    return path


def load_replay(resume):
    packet = resume / "adapter-path-retry/intake.json"
    launcher = packet.with_name("retry.py")
    run.require(run.digest(packet) == INTAKE_SHA256 and run.digest(launcher) == LAUNCHER_SHA256,
                "retry intake or launcher differs from the registered seal")

    # Use the checkout's byte-identical verifier inputs when the local staging
    # tree is absent. Never re-stage the controller's working files.
    original_intake = resume / "intake.json"
    run.require(run.digest(original_intake) == ORIGINAL_INTAKE_SHA256, "original intake differs from the registered seal")
    original_packet = run.read_json(original_intake)
    for name, digest in original_packet["local_sha256"].items():
        run.require(run.digest(sealed_source(name)) == digest, f"original intake input differs: {name}")
    retry_packet = run.read_json(packet)
    for name, digest in retry_packet["local_sha256"].items():
        run.require(run.digest(ROOT / name) == digest, f"retry intake input differs: {name}")
    drivers = {Path(name).name: sealed_source(name) for name in original_packet["local_sha256"]
               if Path(name).is_relative_to(FROZEN) and Path(name).suffix == ".py"}
    sys.path[:0] = list(dict.fromkeys(str(path.parent) for path in drivers.values()))
    # The controller imports run by name. Load its pinned copy before importing
    # the replay modules, even when the viewer and frozen drivers have drifted.
    del sys.modules["run"]
    sys.path.insert(0, str(resume))
    retry = runpy.run_path(str(launcher))
    original = retry["original"]
    recovered = retry["RECOVERED"]
    original.sha = lambda path: run.digest(sealed_source(path.relative_to(ROOT)))
    doc = original.document()
    retry["check"](doc)
    original.timing_plan = retry["plan"]
    return original, doc, recovered, launcher, drivers


def replay(resume, previous_release=None):
    timing, archive, release_sha, live = release_receipt(resume, previous_release)
    original, doc, recovered, launcher, drivers = load_replay(resume)
    failed_timing = timing.parent / "timing-release.json"
    failed_setup = resume / "evidence/setup-transport-failure/raw-original.tar.gz"
    failed_setup_summary = run.read_json(failed_setup.with_name("summary.json"))
    failure = run.read_json(failed_timing)
    run.require(run.digest(failed_setup) == failed_setup_summary["original_archive_sha256"]
                and failed_setup_summary["original_export_bytes"] == 0, "original SETUP export differs")
    with tarfile.open(failed_setup, "r:gz") as capture:
        run.require(capture.getmember("setup/raw.tar.gz").size == 0, "failed SETUP contains an unexpected raw export")
    run.require((failure["id"], failure["phase"]) == (original.ID, "timing")
                and failure["exit_status"] != 0
                and not list((timing.parent / "timing/blocks").glob("*/reconciled.json")),
                "original failed TIMING has a different disposition")
    score = original.scores(doc, recovered, timing)
    if live and previous_release is None:
        command = [sys.executable, "-B", str(launcher), "scores"]
        reported = json.loads(subprocess.check_output(command, cwd=ROOT))
        run.require(reported == score, "sealed retry scores differ from the portable archive replay")

    manifest = run.read_json(timing / "series.json")
    run.require(manifest["drivers"] == {name: run.digest(path) for name, path in drivers.items() if name != "intake.py"},
                "TIMING driver hashes differ from the sealed originals")
    lanes = manifest["config"]["lanes"]
    run.require(manifest["config"]["cpus"] == [0, 1]
                and {key: lane["engine"] for key, lane in lanes.items()} == LANES,
                "expected seven registered lanes on CPUs 0 and 1")
    for label, lane in lanes.items():
        run.require(run.read_json(timing / label / "inventory.json") == lane["inventory"],
                    f"{label}: staged inventory differs from the replayed manifest")
    run.require([group["group"] for group in score["groups"]] == list(range(1, 6)),
                "retry must replay five distinct rotations")
    blocks = [capture_block(timing / "blocks" / group["path"], block, group, lanes)
              for group, block in zip(score["groups"], manifest["plan"], strict=True)]
    return dict(manifest_sha256=run.digest(timing / "series.json"), release_sha256=release_sha,
                source=str(timing.relative_to(ROOT)), release=str(archive.relative_to(ROOT)),
                qualification=score["qualification"], blocks=blocks,
                retry_intake_sha256=INTAKE_SHA256, retry_launcher_sha256=LAUNCHER_SHA256,
                setup_release_sha256=run.digest(recovered / "controller-release.json"),
                original_setup_archive_sha256=run.digest(failed_setup),
                original_timing_release_sha256=run.digest(failed_timing))


def capture_block(attempt, block, group, lanes):
    archives, scores = {}, {}
    prior_settlement = 0
    for phase in block["phases"]:
        for label in block["order"]:
            engine = LANES[label]
            arm = f"{phase}-{label}/{phase}-{engine}"
            raw = attempt / arm / "raw.tar.gz"
            archives[arm] = run.digest(raw)
            with tarfile.open(raw, "r:gz") as capture:
                ledger = run.file_json(capture, "adapter/ledger.json")
                submit = min(row["submitted_us"] for row in ledger["transactions"])
                settlement = max(row["settled_us"] for row in ledger["jobs"])
                run.require(prior_settlement <= submit and len(ledger["jobs"]) == 64,
                            f"{raw}: unexpected execution order or job count")
                prior_settlement = settlement
                if phase == "warmup":
                    continue
                row = group["scores"][label]
                run.require(row["raw_sha256"] == archives[arm] and row["affinity"] == [0, 1]
                            and (row["completed"], row["ordered_independent_writes"], row["failed"])
                            == (64, 64, 0), f"{raw}: archive hash, affinity or durable job count differs")
                runner = lanes[label]["inventory"]["engines"][engine]["artifact_sha256"]
                row = dict(row, engine=engine, sha256=runner,
                           submit_to_last_settlement_ms=group["clocks"][label]["submit_ms"],
                           commit_to_last_settlement_ms=group["clocks"][label]["drain_ms"])
                if engine == "mesh":
                    identities = run.file_json(capture, "adapter/process-identity.json")
                    broker, = (p["executable_sha256"] for p in identities if p["argv"][-1] == "session")
                    row.update(runner_sha256=runner, broker_sha256=broker)
                scores[label] = row
    return dict(group=block["group"], profile=block["profile"], order=block["order"],
                attempt=attempt.name, scores=scores, raw_sha256=archives,
                order_sha256=run.digest(attempt / "order.json"),
                reconciliation_sha256=run.digest(attempt / "reconciled.json"),
                **{key: group[key] for key in ("clocks", "duplicate_spread_ms",
                                               "faster_duplicate_minus_file_ms", "paired_ms")})


if __name__ == "__main__":
    try:
        print(json.dumps(replay(Path(sys.argv[1]), *sys.argv[2:]), sort_keys=True, allow_nan=False))
    except (AssertionError, OSError, KeyError, ValueError, subprocess.CalledProcessError, tarfile.TarError) as error:
        sys.exit(str(error) or "native replay rejected the sealed evidence")
