"""Seal, replay and use the existing grant-fenced fixed8 Linode dispatch."""

from copy import deepcopy
import importlib.util
import json
from pathlib import PurePosixPath
import shlex
import subprocess
import sys
import tarfile

import overlay
import replay
import stage_arm

HERE = overlay.HERE
ROOT = HERE.parents[3]
BASE = HERE.parent / "2026-11-25-dense-fixed8-logical-factorial"
ID = HERE.name
OUT = HERE / "evidence/native"
PACKET = HERE / "controller-packet.json"
LANES = {"stock-a": "stock", "stock-b": "stock", "projected": "projected"}
ORDER = ("stock-a", "stock-b", "projected")
POLICIES = {"combined": "fixed8ms"}

sys.path.insert(0, str(BASE))
spec = importlib.util.spec_from_file_location("qualified_fixed8_factorial", BASE / "controller.py")
factorial = importlib.util.module_from_spec(spec)
spec.loader.exec_module(factorial)
sys.path.remove(str(BASE))
run = factorial.run
original_score = factorial.score_arm
TRIALS = [factorial.block(1, ORDER), factorial.block(2, ORDER[::-1])]
FILES = (
    HERE / "launch.sh", HERE / "stage_arm.py", HERE / "overlay.py", HERE / "enqueue-stock.sql",
    *(path for path in factorial.FILES if path.name != "launch.sh"),
)


def prior():
    base, = (factorial.OUT / "staging").glob(f"{factorial.PEERS_ID}.*")
    inventory = base / "combined/inventory.json"
    release = factorial.OUT / "peers-release.json"
    row = replay.read(release)
    assert row["id"] == factorial.PEERS_ID and row["exit_status"] == 0
    assert row["groups"] == [3, 4, 5, 6, 7] and row["released_at"]
    return inventory, release


def config():
    prior_inventory, _ = prior()
    baseline = replay.read(prior_inventory)
    original = factorial.config()
    assert baseline == original["lanes"]["combined"]["inventory"]
    spec = baseline["engines"]["mesh"]
    argv = spec["argv"]
    assert argv[1] == "JOBS_GROUPED_WRITER=fixed8ms"
    assert argv[-2:] == ["combined", "mesh"] and f"PRODUCT_ELF_SHA256={factorial.ELF}" in argv
    assert replay.sha(factorial.LOGICAL / "candidate.sql") == factorial.SQL_SHA
    assigned = deepcopy(original)
    assigned["lanes"] = {}
    for lane, treatment in LANES.items():
        remote = run.REMOTE / f"{ID}-{lane}"
        inventory = deepcopy(baseline)
        engine = inventory["engines"]["mesh"]
        engine["argv"] = [*argv[:-4], str(remote / "launch.sh"), argv[-3], treatment, "mesh"]
        old_parent = str(PurePosixPath(argv[-4]).parent)
        engine["verified_files"] = {path: digest for path, digest in spec["verified_files"].items()
                                    if not path.startswith(old_parent + "/")}
        pins = (*FILES, factorial.SUITE / "worker.py")
        engine["verified_files"].update({str(remote / path.name): replay.sha(path) for path in pins})
        run.check_inventory(inventory)
        assigned["lanes"][lane] = dict(engine="mesh", runners=2, inventory=inventory)
    return original, assigned


def score(folder, phase, lane, cfg):
    result = original_score(folder, phase, "combined",
                            dict(cfg, lanes={"combined": cfg["lanes"][lane]}),
                            factorial.grouped, factorial.logical, POLICIES)
    with tarfile.open(folder / "raw.tar.gz") as raw:
        counts = run.file_json(raw, "adapter/enqueue-cardinality.json")
        overlay_meta = run.file_json(raw, "adapter/enqueue-overlay.json")
        ledger = run.file_json(raw, "adapter/ledger.json")
        assert counts == {"bulk": [8, 1024]}
        before, after = overlay_meta["before"], overlay_meta["after"]
        assert before["body"] == overlay.digest(overlay.body(overlay.source()))
        sql = overlay.candidate() if LANES[lane] == "projected" else overlay.source()
        assert after == dict(before, body=overlay.digest(overlay.body(sql)))
        assert before["producer"] and before["invoker"] and before["volatile"] and not (
            before["runner"] or before["application"])
        assert before["owner"] == "parity_migrator"
        transaction, = ledger["transactions"]
        jobs = ledger["jobs"]
        assert len(jobs) == 1024 and all(job["attempts"] == job["effects"] == 1 for job in jobs)
        assert len(transaction["enqueued_ids"]) == 1024
    return dict(result, enqueue_us=result["first_commit_us"] - result["first_submit_us"],
                acknowledgement_us=transaction["acknowledged_us"] - transaction["submitted_us"],
                writer_counts=counts, enqueue_overlay=overlay_meta,
                peak_runner_rss_bytes=ledger["peak_runner_rss_bytes"])


def score_block(folder, trial, cfg):
    assert replay.read(folder / "order.json") == trial
    phases = {}
    attempts = set()
    for phase, lane, _, arm in factorial.grouped.matrix.paths(folder, cfg, trial):
        attempt = replay.read(arm / "request.json")["attempt"]
        assert attempt not in attempts
        attempts.add(attempt)
        phases.setdefault(phase, {})[lane] = score(arm, phase, lane, cfg)
    return phases


def gate(rows):
    a, b, candidate = (rows[lane] for lane in ("stock-a", "stock-b", "projected"))
    enqueue_spread = abs(a["enqueue_us"] - b["enqueue_us"])
    enqueue_saved = min(a["enqueue_us"], b["enqueue_us"]) - candidate["enqueue_us"]
    submit_saved = min(a["submit_us"], b["submit_us"]) - candidate["submit_us"]
    drain = min(a["drain_us"], b["drain_us"])
    drain_extra = candidate["drain_us"] - drain
    drain_limit = max(10000, .02 * drain, abs(a["drain_us"] - b["drain_us"]))
    return dict(enqueue_saved_us=enqueue_saved, enqueue_duplicate_spread_us=enqueue_spread,
                submit_saved_us=submit_saved, drain_extra_us=drain_extra, drain_limit_us=drain_limit,
                passed=enqueue_saved > max(10000, enqueue_spread) and submit_saved > 0
                and drain_extra <= drain_limit)


def replay_native(cfg):
    groups = {}
    for trial in TRIALS:
        matches = list((OUT / "blocks").glob(f"{run.block_prefix(trial)}.*"))
        assert len(matches) <= 1, "duplicate block attempts"
        if matches:
            folder, = matches
            scored = score_block(folder, trial, cfg)
            assert replay.read(folder / "reconciled.json") == json.loads(json.dumps(scored))
            groups[trial["group"]] = dict(arms=scored, gate=gate(scored["measured"]))
    release = factorial.QUEUE / f"{ID}-groups-1-2.released.json"
    state = replay.read(release) if release.exists() else None
    valid = bool(state and state["id"] == ID and state["groups"] == [1, 2] and state["exit_status"] == 0
                 and state["receipt"] == str(OUT) and state["released_at"])
    return dict(groups=groups, release=state, passed=valid and len(groups) == 2
                and all(row["gate"]["passed"] for row in groups.values()))


def command(action):
    return shlex.join(("python3", "-B", str((HERE / "controller.py").relative_to(ROOT)), action))


def packet(history):
    sources = {*FILES, *HERE.glob("*.py"), HERE / "candidate.sql", HERE / "README.md",
               replay.SOURCE, *(p for p in replay.EVIDENCE.iterdir() if p.is_file()),
               *prior(), history, BASE / "controller.py", BASE / "score.py",
               factorial.GROUPED / "pilot.py", factorial.LOGICAL / "controller.py",
               factorial.SOURCE, *(factorial.SUITE / n for n in ("run.py", "worker.py", "verify.py", "interleave.py")),
               history.parent / "request.json", history.parent / "dispatch.json"}
    return dict(id=ID, ready=True, no_build=True, after_existing_bookings=True,
                controller="native-controller-complete-post-wal-queue", native_host=factorial.logical.HOST,
                cpus=[0, 1], elf_sha256=factorial.ELF, logical_grant_sha256=factorial.SQL_SHA,
                input_sha256=replay.INPUT_SHA, schedule=TRIALS, timing_window_minutes=14,
                gate=dict(enqueue_floor_us=10000, enqueue_duplicate_spread=True,
                          submit="beat both controls", drain_floor_us=10000,
                          drain_fraction=.02, drain_duplicate_spread=True),
                offline_check=command("check"), timing_command=command("timing"), replay_command=command("replay"),
                evidence_destination=str(OUT), controller_receipt_dir=str(OUT / "controller"),
                local_sha256={str(p.relative_to(ROOT)): replay.sha(p) for p in sorted(sources)})


def preflight(original, cfg):
    replay.verify()
    assert replay.sha(factorial.SOURCE) == replay.SOURCE_SHA
    subprocess.run(["bash", "-n", HERE / "launch.sh"], check=True)
    base = (BASE / "factorial_arm.py").read_text()
    stock, treatment = (stage_arm.render(base, mode) for mode in ("stock", "projected"))
    assert stock.replace("overlay.apply_writer(cluster, False)", "overlay.apply_writer(cluster, True)") == treatment
    native = PurePosixPath(cfg["lanes"]["stock-a"]["inventory"]["engines"]["mesh"]["argv"][-3])
    with tarfile.open(replay.SOURCE) as archive:
        native_run = archive.extractfile("source/" + str(native.relative_to(native.parents[5]))).read().decode()
    launcher = factorial.render(native_run, native.parents[5], "combined")
    assert launcher.count('"$here/factorial_arm.py" combined "$@"') == 2
    subprocess.run(["bash", "-n"], input=launcher, text=True, check=True)
    trial = factorial.PEER_TRIALS[0]
    folder, = (factorial.OUT / "blocks").glob(f"{run.block_prefix(trial)}.*")
    folder = folder / "measured-combined/measured-mesh"
    result = original_score(folder, "measured", "combined", original, factorial.grouped,
                            factorial.logical, POLICIES)
    assert result["completed"] == 1024
    raw = folder / "raw.tar.gz"
    with tarfile.open(raw) as archive:
        writer = run.file_json(archive, "adapter/producer-writer.json")
        assert writer["bulk"] == 8 and writer["scalar"] == 0
    control = {"enqueue_us": 150000, "submit_us": 1000000, "drain_us": 850000}
    slower = {"enqueue_us": 165000, "submit_us": 1020000, "drain_us": 855000}
    at_limit = {"enqueue_us": 135000, "submit_us": 998000, "drain_us": 863000}
    rows = {"stock-a": control, "stock-b": slower, "projected": at_limit}
    assert not gate(rows)["passed"]
    assert gate(dict(rows, projected=dict(at_limit, enqueue_us=134999)))["passed"]
    assert not gate(dict(rows, projected=dict(at_limit, enqueue_us=134999, submit_us=1000000)))["passed"]
    assert not gate(dict(rows, projected=dict(at_limit, enqueue_us=134999, drain_us=867001)))["passed"]
    return raw


def bind_dispatch():
    factorial.ID = ID
    factorial.OUT = OUT
    factorial.PACKET = PACKET
    factorial.PRIMARY = TRIALS
    factorial.FILES = FILES
    factorial.POLICIES = dict.fromkeys(LANES, "fixed8ms")
    factorial.score_arm = lambda folder, phase, lane, cfg, *_: score(folder, phase, lane, cfg)
    factorial.score_block = score_block


def main(args):
    original, cfg = config()
    expected = packet(preflight(original, cfg))
    if args == ["seal"]:
        with PACKET.open("x") as stream:
            json.dump(expected, stream, indent=2, sort_keys=True)
            stream.write("\n")
    assert replay.read(PACKET) == expected, "sealed inputs drifted"
    if args in (["seal"], ["check"]):
        print(replay.sha(PACKET))
        return 0
    if args == ["replay"]:
        print(json.dumps(replay_native(cfg), indent=2, sort_keys=True, allow_nan=False))
        return 0
    bind_dispatch()
    if args == ["timing"]:
        return factorial.dispatch(cfg, False)
    if len(args) == 2 and args[0] == "intake":
        return factorial.intake(cfg, args[1], False)
    raise SystemExit("Use check, replay, or controller-only intake PACKET_SHA.")


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
