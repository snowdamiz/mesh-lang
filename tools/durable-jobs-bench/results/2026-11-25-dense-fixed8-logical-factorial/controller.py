"""Build-free receipt, dispatch and replay for the dense fixed8 logical factorial."""

from copy import deepcopy
from datetime import datetime, timezone
import fcntl
import json
from pathlib import Path, PurePosixPath
import re
import runpy
import shlex
import subprocess
import sys
import tarfile
import time

HERE = Path(__file__).resolve().parent
RESULTS = HERE.parent
SUITE = RESULTS.parent
ROOT = SUITE.parent.parent
ORIGINAL_ID = HERE.name
ID = ORIGINAL_ID + "-retry"
PEERS_ID = ID + "-peers"
OUT = HERE / "evidence/native-retry"
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
PACKET = HERE / "controller-packet-retry.json"
PREFLIGHT = HERE / "preflight-retry.json"
GROUPED = RESULTS / "2026-11-25-dense-grouped-refill"
GROUPED_PILOT = GROUPED / "evidence/native-pilot"
GROUPED_SETUP = GROUPED / "evidence/native-setup"
LOGICAL = RESULTS / "2026-11-25-dense-logical-grant-key-native"
LOGICAL_NATIVE = LOGICAL / "evidence/native"
SOURCE = ROOT / "target/dense-grouped-refill-packet/source.tar.gz"
ELF = "d56d0e6112a94ff601bebf6f777061068ed2fe0f2f2da586963dd6e1326a220a"
SOURCE_SHA = "deb2a3bc49feae7df985fe895209c3db7d725cd4c9c1b9c8ba49c93a650dd6d8"
SQL_SHA = "d388baa3b67c07df1a2498cd1fdc60bf7d0ce11f4b42d950f7b0a64a551c0f45"
SQL_FILES = tuple(LOGICAL / name for name in ("contract.py", "stock.sql", "candidate.sql"))
FILES = tuple(HERE / name for name in ("launch.sh", "render.py", "factorial_arm.py")) + SQL_FILES
sys.path[:0] = [str(GROUPED), str(LOGICAL)]
import pilot as grouped
import controller as logical
from contract import STOCK
from render import render
from score import (FLOOR, INCREMENT_US, METRICS, assigned_inventory, contrast, overlay,
                   peer_pairs, score_arm, verify_slots)

run, transfer = grouped.run, grouped.public
PEERS = transfer.PEERS
POLICIES = dict.fromkeys(("stock-a", "stock-b", "key", "smoke-stock"), "") | dict.fromkeys(
    ("fixed8", "combined", "smoke-combined"), "fixed8ms")


def block(group, order, phases=("warmup", "measured")):
    return dict(group=group, order=list(order), phases=list(phases), profile="noop")


ORDER = ("stock-a", "key", "combined", "fixed8", "stock-b")
PRIMARY = [block(0, ("smoke-stock", "smoke-combined"), ("diagnostic",)),
           block(1, ORDER), block(2, ORDER[::-1])]
# The first four orders balance positions and immediate predecessors. The
# fifth is a preselected reverse of the opening order.
CYCLE = ("combined", *PEERS)
PEER_TRIALS = [block(i + 3, (CYCLE[(i + offset) % 4] for offset in (0, 1, 3, 2))) for i in range(4)]
PEER_TRIALS.append(block(7, PEER_TRIALS[0]["order"][::-1]))


def remote(lane):
    return run.REMOTE / f"{ID}-{lane}"


def prior_inventory(label):
    if label in PEERS:
        return logical.inventory_path(label)
    return GROUPED_PILOT / "staging" / label / "inventory.json"


def config():
    cfg = run.read_json(GROUPED_PILOT / "series.json")["config"]
    assert cfg["cpus"] == [0, 1] and cfg["host"] == logical.HOST
    cfg["lanes"] = {}
    for lane in (*POLICIES, *PEERS):
        is_mesh = lane in POLICIES
        engine = "mesh" if is_mesh else {"river-2x32": "go", "oban-2x32": "oban", "apalis-2x32": "rust"}[lane]
        base = "fixed8" if POLICIES.get(lane) else "stock-a"
        inventory = run.read_json(prior_inventory(base if is_mesh else lane))
        spec = inventory["engines"][engine]
        pins = spec["verified_files"]
        worker, = [name for name in pins if name.startswith(str(run.REMOTE) + "/") and name.endswith("/worker.py")]
        assert pins.pop(worker) == run.digest(SUITE / "worker.py")
        pins[str(remote(lane) / "worker.py")] = run.digest(SUITE / "worker.py")
        if is_mesh:
            argv = spec["argv"]
            assert argv[1] == f"JOBS_GROUPED_WRITER={POLICIES[lane]}" and argv[-1] == "mesh"
            assert f"PRODUCT_ELF_SHA256={ELF}" in argv
            spec["argv"] = [*argv[:-2], "/bin/bash", str(remote(lane) / "launch.sh"), argv[-2], lane, "mesh"]
            pins.update({str(remote(lane) / source.name): run.digest(source) for source in FILES})
        run.check_inventory(inventory)
        cfg["lanes"][lane] = dict(engine=engine, runners=2, inventory=inventory)
    return cfg


def cmd(action):
    return shlex.join(("python3", "-B", str((HERE / "controller.py").relative_to(ROOT)), action))


def packet():
    sources = [*FILES, *HERE.glob("*.py"), HERE / "README.md", SOURCE,
               HERE / "controller-packet.json", *original_smoke_files(),
               GROUPED / "request.json", GROUPED / "candidate.patch", GROUPED / "package.py", GROUPED / "pilot.py",
               *(GROUPED_SETUP / name for name in ("raw.tar.gz", "controller-release.json", "binding.json")),
               GROUPED_PILOT / "series.json", GROUPED_PILOT / "controller-release.json",
               LOGICAL / "controller-packet.json", LOGICAL / "controller.py", LOGICAL / "proof.json",
               LOGICAL_NATIVE / "SHA256SUMS", transfer.HERE / "transfer.py",
               *(prior_inventory(lane) for lane in ("stock-a", "stock-b", "fixed8", *PEERS)),
               *(SUITE / name for name in ("interleave.py", "run.py", "verify.py", "worker.py"))]
    return dict(id=ID, ready=True, no_build=True, after_existing_bookings=True,
                controller="native-controller-complete-post-wal-queue",
                supersedes_attempt=ORIGINAL_ID,
                correction="Receipt slots is total seats; runner logs still enforce 32 each.",
                native_host=logical.HOST, cpus=[0, 1], input_sha256=transfer.INPUT,
                candidate_sha256=SQL_SHA, elf_sha256=ELF,
                gate=dict(faster_duplicate_fraction=FLOOR, use_duplicate_spread=True,
                          incremental_floor_us=INCREMENT_US, candidate="combined",
                          max_cpu="higher_duplicate", smoother="none"),
                offline_check=cmd("check"), timing_command=cmd("timing"),
                peer_command=cmd("peers"), replay_command=cmd("replay"),
                schedule=PRIMARY, peer_schedule=PEER_TRIALS,
                timing_window_minutes=18, peer_window_minutes=26,
                evidence_destination=str(OUT), controller_receipt_dir=str(OUT / "controller"),
                local_sha256={str(path.relative_to(ROOT)): run.digest(path) for path in sorted(set(sources))})


def score_block(path, trial, cfg):
    assert run.read_json(path / "order.json") == trial
    phases = {}
    attempts = set()
    for phase, lane, lane_config, folder in grouped.matrix.paths(path, cfg, trial):
        attempt = run.read_json(folder / "request.json")["attempt"]
        assert attempt not in attempts, "duplicate native attempt"
        attempts.add(attempt)
        phases.setdefault(phase, {})[lane] = score_arm(folder, phase, lane, cfg, grouped, logical, POLICIES)
    return phases


def replay(cfg):
    groups, failures, repeated = {}, {}, {}
    raw_custody = {str(path.relative_to(OUT)): run.digest(path)
                   for path in sorted(OUT.rglob("raw.tar.gz"))}
    for trial in PRIMARY + PEER_TRIALS:
        group = trial["group"]
        for path in sorted((OUT / "blocks").glob(f"{run.block_prefix(trial)}.*")):
            try:
                rows = score_block(path, trial, cfg)
                assert run.read_json(path / "reconciled.json") == json.loads(json.dumps(rows))
            except (AssertionError, RuntimeError, OSError, ValueError, KeyError, StopIteration, tarfile.TarError) as error:
                failures[path.name] = f"{type(error).__name__}: {error}"
            else:
                if group in groups:
                    repeated[path.name] = rows
                else:
                    groups[group] = rows
    gates = {group: contrast(groups[group]["measured"]) for group in (1, 2) if group in groups}
    passed = not failures and not repeated and {0, 1, 2} <= groups.keys() and all(
        gate["passed"] for gate in gates.values())
    pairs = peer_pairs([groups[i] for i in range(3, 8)], PEERS) if set(range(3, 8)) <= groups.keys() else None
    return dict(groups=groups, failures=failures, repeated=repeated, raw_sha256=raw_custody,
                pilot_gates=gates, peer_eligible=passed, fresh_peer_pairs=pairs)


def group_release(key, first, last):
    return QUEUE / f"{key}-groups-{first}-{last}.released.json"


def clean_primary(cfg):
    receipt = run.read_json(group_release(ID, 0, 2))
    assert receipt["id"] == ID and receipt["groups"] == [0, 1, 2]
    assert receipt["exit_status"] == 0 and receipt["receipt"] == str(OUT) and receipt["released_at"]
    assert replay(cfg)["peer_eligible"], "both clean prospective pilot gates must pass"


def active(key):
    assert (QUEUE / f"{key}.json").read_bytes() == PACKET.read_bytes(), "registered packet differs"
    journal = (QUEUE / "STATUS.md").read_text()
    kind, name, start, end = re.findall(
        r"^(SETUP|TIMING|RELEASED|HOLD) (\S+)(?: ([^\s,]+))?(?: ([^\s,]+))?", journal, re.M)[-1]
    assert (kind, name) == ("TIMING", key)
    assert datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
    return dict(id=key, start=start, end=end, request_sha256=run.digest(PACKET))


def dispatch(cfg, peers):
    from interleave import HOST_IDLE

    key, trials = (PEERS_ID, PEER_TRIALS) if peers else (ID, PRIMARY)
    if peers:
        clean_primary(cfg)
    grant = active(key)
    OUT.mkdir(parents=True, exist_ok=True)
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        active(key)
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
        nonce = time.time_ns()
        run.write_json(OUT / f"grant-{nonce}.json", grant)
        staging = OUT / "staging" / f"{key}.{nonce}"
        for lane in sorted({lane for trial in trials for lane in trial["order"]}):
            folder = staging / lane
            folder.mkdir(parents=True)
            inventory = folder / "inventory.json"
            run.write_json(inventory, cfg["lanes"][lane]["inventory"])
            files = (FILES if lane in POLICIES else ()) + (SUITE / "worker.py", inventory)
            with (folder / "stage.log").open("xb") as log:
                for source in files:
                    active(key)
                    run.stage(source, remote(lane), log)
        for trial in trials:
            path = OUT / "blocks" / f"{run.block_prefix(trial)}.{time.time_ns()}"
            path.mkdir(parents=True)
            run.write_json(path / "order.json", trial)
            for phase, lane, lane_config, folder in grouped.matrix.paths(path, cfg, trial):
                end = datetime.fromisoformat(active(key)["end"])
                assert (end - datetime.now(timezone.utc)).total_seconds() >= 125, "grant time budget exhausted"
                folder.parent.mkdir()
                run.dispatch_arm(folder.parent, remote(lane), trial, phase, lane_config["engine"])
                score_arm(folder, phase, lane, cfg, grouped, logical, POLICIES)
            run.write_json(path / "reconciled.json", score_block(path, trial, cfg))
    return 0


def intake(cfg, seal, peers):
    assert seal == run.digest(PACKET), "exact packet hash required"
    if peers:
        clean_primary(cfg)
    key = PEERS_ID if peers else ID
    groups = [trial["group"] for trial in (PEER_TRIALS if peers else PRIMARY)]
    assert (QUEUE / f"{key}.json").read_bytes() == PACKET.read_bytes(), "controller registration required"
    released = group_release(key, groups[0], groups[-1])
    if released.is_file():
        return run.read_json(released)["exit_status"]
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) ", (QUEUE / "STATUS.md").read_text(), re.M)
    assert events[-1] == "RELEASED", "earlier work still owns the native controller"
    request = run.read_json(PACKET)
    request.update(timing_grant_id=key, command_attempts=1,
                   timing_window_minutes=request["peer_window_minutes" if peers else "timing_window_minutes"])
    return runpy.run_path(str(QUEUE.parent / "completion-writer-r2.py"))["measure"](
        request, "peer_command" if peers else "timing_command", groups)


def git_bytes(revision, path):
    return subprocess.check_output(["git", "show", f"{revision}:{path.relative_to(ROOT)}"], cwd=ROOT)


def assert_release(path, identifier):
    receipt = run.read_json(path)
    assert receipt["id"] == identifier and receipt["exit_status"] == 0 and receipt["released_at"]
    return receipt


def source_check(cfg):
    assert run.digest(SOURCE) == SOURCE_SHA
    assert run.digest(LOGICAL / "candidate.sql") == SQL_SHA
    for path in (*SQL_FILES, LOGICAL / "proof.json", LOGICAL_NATIVE / "SHA256SUMS"):
        assert path.read_bytes() == git_bytes("7f6d84a31", path), f"released logical input drift: {path}"
    assert_release(GROUPED_PILOT / "controller-release.json", GROUPED.name + "-pilot")
    assert (GROUPED_SETUP / "binding.json").read_bytes() == git_bytes("bb79fee6a", GROUPED_SETUP / "binding.json")
    binding = grouped.setup(SOURCE, GROUPED_SETUP / "controller-release.json", GROUPED_SETUP / "raw.tar.gz")
    assert binding == run.read_json(GROUPED_SETUP / "binding.json") == cfg["product"]
    assert binding["build"]["files"]["mesh"] == ELF
    logical.source_check()
    for line in (LOGICAL_NATIVE / "SHA256SUMS").read_text().splitlines():
        checksum, rel = line.split("  ", 1)
        assert run.digest(LOGICAL_NATIVE / rel) == checksum, f"released logical receipt drift: {rel}"
    release, = (LOGICAL_NATIVE / "controller").glob("controller-timing-*.json")
    assert_release(release, LOGICAL.name)

    def strip_options(spec):
        result = deepcopy(spec)
        del result["argv"][1]
        pins = result["verified_files"]
        worker, = [name for name in pins if name.startswith(str(run.REMOTE) + "/") and name.endswith("/worker.py")]
        del pins[worker]
        return result

    old = [run.read_json(prior_inventory(lane))["engines"]["mesh"]
           for lane in ("stock-a", "stock-b", "fixed8")]
    assert strip_options(old[0]) == strip_options(old[1]) == strip_options(old[2])
    native = PurePosixPath(old[0]["argv"][-2])
    assert native.parents[5] == transfer.BASE / f"source-trusted-jobs-product-{SOURCE_SHA[:12]}/source"
    with tarfile.open(SOURCE) as archive:
        get = transfer.archive_files(archive, "source/")
        schema = get("compiler/mesh-rt/src/jobs/schema.sql")
        assert schema.count(STOCK.encode()) == 1
        for name in ("native-run", "product_arm.py", "backend.py", "identity.py"):
            target = native.with_name(name)
            data = get(str(target.relative_to(native.parents[5])))
            assert transfer.digest(data) == old[0]["verified_files"][str(target)]
            if name.endswith(".py"):
                compile(data, name, "exec")
        source = get(str(native.relative_to(native.parents[5]))).decode()
    for lane in POLICIES:
        spec = cfg["lanes"][lane]["inventory"]["engines"]["mesh"]
        assert spec["argv"][-3:] == [str(native), lane, "mesh"]
        staged = render(source, native.parents[5], lane)
        subprocess.run(["bash", "-n"], input=staged, text=True, check=True)
        for must_survive in ("flock -n 9", "runuser -u nobody", "env -i", "JOBS_GROUPED_WRITER=", "PRODUCT_FROZEN_HARNESS="):
            assert must_survive in staged
        assert staged.count(f'"$here/factorial_arm.py" {lane} "$@"') == 2
    subprocess.run(["bash", "-n", HERE / "launch.sh"], check=True)
    for path in (*HERE.glob("*.py"), LOGICAL / "contract.py"):
        compile(path.read_bytes(), str(path), "exec")
    return binding


def original_smoke_files():
    prior = HERE / "evidence/native"
    raw, = (prior / "blocks").glob("00-noop.*/diagnostic-smoke-stock/diagnostic-mesh/raw.tar.gz")
    inventory, = (prior / "staging").glob("*/smoke-stock/inventory.json")
    release, = (prior / "controller").glob("controller-timing-*.json")
    return raw, inventory, release


def dry_score(cfg):
    # Reconcile the original writer evidence, the exact SQL/CPU evidence and
    # actual peer inventories independently. These arms test replay only.
    results = {}
    stock_raw, stock_inventory, release = original_smoke_files()
    first = run.read_json(release)
    assert first["id"] == ORIGINAL_ID and first["exit_status"] == 1 and first["released_at"]
    first_cfg = deepcopy(cfg)
    first_cfg["lanes"]["smoke-stock"]["inventory"] = run.read_json(stock_inventory)
    stock = score_arm(stock_raw.parent, "diagnostic", "smoke-stock", first_cfg, grouped, logical, POLICIES)
    results[str(stock_raw.relative_to(ROOT))] = stock["raw_sha256"]
    for trial in grouped.SCHEDULE:
        original, = (GROUPED_PILOT / "blocks").glob(f"{run.block_prefix(trial)}.*")
        assert run.read_json(original / "order.json") == trial
        for phase in trial["phases"]:
            for lane in ("stock-a", "fixed8"):
                folder = original / f"{phase}-{lane}" / f"{phase}-mesh"
                raw = folder / "raw.tar.gz"
                assert raw.read_bytes() == git_bytes("bb79fee6a", raw)
                with tarfile.open(raw) as archive:
                    verify_slots(run.file_json(archive, "adapter/ledger.json"),
                                 run.file_json(archive, "adapter/row.json"))
                assert run.read_json(folder / "dispatch.json")["exit_code"] == 0
                inventory = run.read_json(prior_inventory(lane))
                with assigned_inventory(transfer, inventory):
                    row = grouped.score_arm(folder, phase, lane, dict(cfg, lanes={"hot": cfg["lanes"][lane]}), None)
                results[str(raw.relative_to(ROOT))] = row["raw_sha256"]
    prior_cfg = logical.check()
    for trial in logical.PRIMARY:
        original, = (LOGICAL_NATIVE / "blocks").glob(f"{run.block_prefix(trial)}.*")
        phases = logical.score_block(original, trial, prior_cfg)
        for lane, row in phases["measured"].items():
            folder = original / f"measured-{lane}/measured-mesh"
            raw = folder / "raw.tar.gz"
            assert row["raw_sha256"] == run.digest(raw)
            if trial["group"]:
                with tarfile.open(raw) as archive:
                    overlay(run.file_json(archive, "adapter/logical-overlay.json"), lane,
                            logical.CANDIDATE.read_text() if lane == "key" else STOCK)
            results[str(raw.relative_to(ROOT))] = row["raw_sha256"]
    original, = (logical.SERIES / "blocks").glob("01-noop.*")
    for phase in ("warmup", "measured"):
        for lane in PEERS:
            engine = cfg["lanes"][lane]["engine"]
            folder = original / f"{phase}-{lane}" / f"{phase}-{engine}"
            inventory = run.read_json(prior_inventory(lane))
            peer_cfg = deepcopy(cfg)
            peer_cfg["lanes"][lane]["inventory"] = inventory
            row = score_arm(folder, phase, lane, peer_cfg, grouped, logical, POLICIES)
            results[str((folder / "raw.tar.gz").relative_to(ROOT))] = row["raw_sha256"]
    return results


def gate_check():
    def row(clock, cpu, xids):
        return {name: value for name, value in zip(METRICS, (clock, clock, clock, cpu, cpu, cpu, xids))}
    rows = dict(zip(ORDER, (row(1_010_000, 70, 850), row(910_000, 70, 850),
                            row(880_000, 68, 183), row(905_000, 70, 183), row(1_000_000, 71, 860))))
    assert contrast(rows)["passed"]
    assert not contrast(dict(rows, combined=row(895_000, 68, 183)))["passed"]
    assert not contrast(dict(rows, combined=row(880_000, 72, 183)))["passed"]
    assert not contrast(dict(rows, combined=row(880_000, 68, 850)))["passed"]
    assert not contrast(dict(rows, fixed8=row(875_000, 70, 183)))["passed"]
    assert not contrast(dict(rows, **{"stock-a": row(1_125_000, 70, 850)}))["passed"]


def main(args):
    cfg = config()
    expected = packet()
    assert PRIMARY[1]["order"] == PRIMARY[2]["order"][::-1]
    assert all(set(trial["order"]) == {"combined", *PEERS} for trial in PEER_TRIALS)
    if args == ["seal"]:
        binding = source_check(cfg)
        gate_check()
        custody = dry_score(cfg)
        run.write_json(PREFLIGHT, dict(original_raw_archives=custody, grouped_binding=binding))
        run.write_json(PACKET, expected)
        print(run.digest(PACKET))
        return 0
    assert run.read_json(PACKET) == expected, "sealed dependency or packet drift"
    if args == ["check"]:
        binding = source_check(cfg)
        gate_check()
        assert run.read_json(PREFLIGHT) == dict(
            original_raw_archives=dry_score(cfg), grouped_binding=binding)
        print(f"{ID} verified {run.digest(PACKET)}")
        return 0
    if args == ["replay"]:
        print(json.dumps(replay(cfg), indent=2, sort_keys=True, allow_nan=False))
        return 0
    if args in (["timing"], ["peers"]):
        return dispatch(cfg, args == ["peers"])
    if len(args) == 2 and args[0] in {"intake", "intake-peers"}:
        return intake(cfg, args[1], args[0] == "intake-peers")
    raise SystemExit("Use seal, check, replay, intake PACKET_SHA or intake-peers PACKET_SHA.")


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
