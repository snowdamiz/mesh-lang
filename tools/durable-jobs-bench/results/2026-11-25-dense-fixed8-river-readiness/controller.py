"""Offline replay and grant-fenced native dispatch for the dense readiness sensitivity."""

from copy import deepcopy
from datetime import datetime, timedelta, timezone
import fcntl
import importlib.util
import json
import re
import runpy
import shlex
import subprocess
import sys
import tarfile
import time

import build
import metrics

HERE, ROOT, SUITE = build.HERE, build.ROOT, build.SUITE
ID = HERE.name
OUT = build.EVIDENCE / "native"
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
PACKET = HERE / "controller-packet.json"
HISTORY = build.EVIDENCE / "stock-peer-replay.json"
FACTORIAL = HERE.parent / "2026-11-25-dense-fixed8-logical-factorial"
ORIGINAL = build.DENSE / "evidence/equal"

sys.path.insert(0, str(FACTORIAL))
spec = importlib.util.spec_from_file_location("qualified_factorial", FACTORIAL / "controller.py")
factorial = importlib.util.module_from_spec(spec)
spec.loader.exec_module(factorial)
sys.path.remove(str(FACTORIAL))
run = factorial.run
from interleave import HOST_IDLE

MODES = {"river-cold": "cold", "river-warm": "warm"}
LANES = ("combined", "river-2x32", *MODES)
ORDERS = (
    LANES,
    ("river-2x32", "river-warm", "combined", "river-cold"),
    ("river-warm", "combined", "river-2x32", "river-cold"),
)
SETUP = [factorial.block(0, LANES, ("diagnostic",))]
TIMING = [factorial.block(i, order) for i, order in
          enumerate((order for pair in ORDERS for order in (pair, pair[::-1])), 1)]
STAGES = {"setup": (SETUP, 12), "pilot": (TIMING[:2], 18), "rest": (TIMING[2:], 32)}


def require_release(path, identifier, groups):
    receipt = build.read(path)
    assert receipt["id"] == identifier and receipt["groups"] == groups
    assert receipt["exit_status"] == 0 and receipt["released_at"]
    return receipt


def prior_inputs():
    pattern = f"{factorial.PEERS_ID}.*"
    staging, = (factorial.OUT / "staging").glob(pattern)
    paths = {lane: staging / lane / "inventory.json" for lane in ("combined", *factorial.PEERS)}
    release_dir = factorial.OUT / "controller"
    releases = [path for path in release_dir.glob("controller-timing-*.json")
                if build.read(path)["id"] == factorial.PEERS_ID]
    release, = releases
    require_release(release, factorial.PEERS_ID, list(range(3, 8)))
    cfg = factorial.config()
    assert all(build.read(path) == cfg["lanes"][lane]["inventory"] for lane, path in paths.items())
    return cfg, paths, release


def original_river(cfg):
    original = ORIGINAL / "staging/river-2x32/inventory.json"
    frozen = build.read(original)["engines"]["go"]
    current = cfg["lanes"]["river-2x32"]["inventory"]["engines"]["go"]
    def without_worker(spec):
        return dict(spec, verified_files={name: sha for name, sha in spec["verified_files"].items()
                                          if not name.endswith("/worker.py")})
    assert without_worker(current) == without_worker(frozen)
    return original, frozen


def details(path, engine, score, ticks, mode=None):
    with tarfile.open(path) as archive:
        assert len(archive.getnames()) == len(set(archive.getnames()))
        ledger = metrics.read(archive, "adapter/ledger.json")
        detail = metrics.ledger_metrics(ledger, score)
        detail.update(raw_sha256=run.digest(path), connections=metrics.samples(archive, engine),
                      ledger_sha256=build.sha(archive.extractfile("adapter/ledger.json").read()))
        if engine == "go":
            cfg = metrics.read(archive, "adapter/river/config.json")
            db = metrics.read(archive, "adapter/river/database.json")
            assert cfg["profile_config"] == dict(runners=2, slots=64, physical_connection_cap=96,
                                                 full_unmeasured_warmup=True, input_sha256=metrics.INPUT)
            assert all(cfg[k] == v for k, v in dict(runner_pool_max=68, producer_observer_pool_max=1,
                       gomaxprocs=2, visible_cpus=2, fetch_cooldown_ms=5, poll_ms=250, idle_ms=3000,
                       poll_only=False, os="linux", arch="amd64", submit_boundary="before_batch_read").items())
            assert all(db[k] == v for k, v in dict(server_version_num=170011, tls=True, user="parity_subject",
                       synchronous_commit=True, fsync=True, full_page_writes=True, track_commit_timestamp=True,
                       max_connections=96, database_connection_limit=96, reserved_connections=0,
                       superuser_reserved_connections=0, other_client_backends=0).items())
            if mode:
                startup = metrics.startup(archive, mode, score, ticks)
                assert detail["connections"]["last_idle"] == startup["physical_at_submit"]
                detail["startup"] = startup
        if engine == "mesh":
            roster = metrics.read(archive, "adapter/postgres-tls-sessions.json")
            assert sum(row["role"] == "compiled_caller" and row["tls"] for row in roster) == 32
            detail["idle_application_tls"] = 32
    return detail


def history():
    cfg, paths, release = prior_inputs()
    original, frozen = original_river(cfg)
    original_series = build.read(ORIGINAL / "series.json")
    rows, pins = {}, [*paths.values(), release, original, ORIGINAL / "series.json",
                     ORIGINAL / "controller-release.json"]
    assert build.read(ORIGINAL / "controller-release.json")["exit_status"] == 0
    eq = {}
    for trial in original_series["schedule"]:
        group = trial["group"]
        candidates = sorted((ORIGINAL / "blocks").glob(f"{run.block_prefix(trial)}.*"), reverse=True)
        raw, = [p for path in candidates
                for p in path.glob("measured-river-2x32/measured-go/raw.tar.gz")]
        result = factorial.transfer.base_score(raw.parent, "go", 2, "measured",
                                               build.read(original), original_series["config"])
        eq[group] = details(raw, "go", result, cfg["clock_ticks"])
    assert len(eq) == 5
    for trial in factorial.PEER_TRIALS:
        path, = (factorial.OUT / "blocks").glob(f"{run.block_prefix(trial)}.*")
        scored = factorial.score_block(path, trial, cfg)
        recorded = path / "reconciled.json"
        assert build.read(recorded) == json.loads(json.dumps(scored))
        pins.append(recorded)
        phases = {}
        for phase, lane, assigned, folder in factorial.grouped.matrix.paths(path, cfg, trial):
            row = details(folder / "raw.tar.gz", assigned["engine"], scored[phase][lane], cfg["clock_ticks"])
            phases.setdefault(phase, {})[lane] = row
        rows[trial["group"]] = dict(order=trial["order"], phases=phases)
    river = [row["phases"]["measured"]["river-2x32"]["connections"] for row in rows.values()]
    assert len(rows) == len(river) == 5
    assert [connection["last_idle"] for connection in river] == [7] * 5
    assert [connection["peak"] for connection in river] == [69, 68, 69, 68, 67]
    return dict(release=build.read(release), original_river_spec=frozen,
                original_equal_measured_river=eq, qualified_combined_with_stock_peers=rows,
                dependency_sha256={str(path.relative_to(ROOT)): run.digest(path) for path in sorted(pins)})


def remote(lane):
    return factorial.remote(lane) if lane not in MODES else run.REMOTE / f"{ID}-{lane}"


def binary_path(receipt):
    return f"/var/lib/durable-jobs-bench/river/builds/dense-readiness-{receipt['candidate_elf'][:16]}/river-bench"


def config(receipt):
    cfg, _, _ = prior_inputs()
    cfg["lanes"] = {lane: cfg["lanes"][lane] for lane in LANES[:2]}
    _, stock = original_river(cfg)
    assert stock["artifact_sha256"] == receipt["stock_elf"]
    path = binary_path(receipt)
    for lane, mode in MODES.items():
        template = deepcopy(cfg["lanes"]["river-2x32"])
        spec = template["inventory"]["engines"]["go"]
        argv = spec["argv"]
        index = argv.index("--binary") + 1
        assert spec["verified_files"][argv[index]] == receipt["stock_elf"]
        argv[index] = path
        spec["argv"] = ["/usr/bin/env", f"RIVER_BENCH_POOL_READINESS={mode}", *argv]
        spec["artifact_sha256"] = receipt["candidate_elf"]
        spec["verified_files"][path] = receipt["candidate_elf"]
        pins = spec["verified_files"]
        old_worker = str(factorial.remote("river-2x32") / "worker.py")
        pins[str(remote(lane) / "worker.py")] = pins.pop(old_worker)
        run.check_inventory(template["inventory"])
        cfg["lanes"][lane] = template
    return cfg


def score_arm(folder, phase, lane, cfg):
    engine = cfg["lanes"][lane]["engine"]
    if lane in MODES:
        assert build.read(folder / "dispatch.json")["exit_code"] == 0
        assigned = cfg["lanes"][lane]["inventory"]
        score = factorial.transfer.base_score(folder, "go", 2, phase, assigned, cfg)
    else:
        score = factorial.score_arm(folder, phase, lane, cfg, factorial.grouped, factorial.logical, factorial.POLICIES)
    detail = details(folder / "raw.tar.gz", engine, score, cfg["clock_ticks"], MODES.get(lane))
    if engine == "go":
        idle, peak = (detail["connections"][key] for key in ("last_idle", "peak"))
        if lane == "river-warm":
            assert idle == 67 and 67 <= peak <= 71
        else:
            assert idle < 34 and peak >= idle + 24
    return detail


def replay_block(path, trial, cfg):
    assert build.read(path / "order.json") == trial
    rows = {}
    attempts = set()
    for phase, lane, _, folder in factorial.grouped.matrix.paths(path, cfg, trial):
        attempt = build.read(folder / "request.json")["attempt"]
        assert attempt not in attempts
        attempts.add(attempt)
        rows.setdefault(phase, {})[lane] = score_arm(folder, phase, lane, cfg)
    return rows


def release_path(stage):
    if stage == "setup":
        return OUT / "controller/setup-release.json"
    return OUT / "controller" / f"{stage}-release.json"


def require_phase(stage, cfg):
    trials, _ = STAGES[stage]
    require_release(release_path(stage), f"{ID}-{stage}", [trial["group"] for trial in trials])
    rows = {}
    for trial in trials:
        path, = (OUT / "blocks").glob(f"{run.block_prefix(trial)}.*")
        row = replay_block(path, trial, cfg)
        assert row == build.read(path / "reconciled.json")
        rows[trial["group"]] = row
    return rows


def command(*args):
    return shlex.join(("python3", "-B", str((HERE / "controller.py").relative_to(ROOT)), *args))


def packet(receipt):
    files = [*HERE.glob("*.py"), HERE / "adapter.patch", HERE / "README.md", build.BINARY, build.RECEIPT, HISTORY,
             HERE / "install.sh", build.DENSE / "river-build.json",
             FACTORIAL / "controller-packet-retry.json", FACTORIAL / "controller.py", FACTORIAL / "score.py",
             *(SUITE / name for name in ("worker.py", "run.py", "verify.py", "interleave.py"))]
    for name, expected in build.read(FACTORIAL / "controller-packet-retry.json")["local_sha256"].items():
        if not name.startswith("target/"):
            assert run.digest(ROOT / name) == expected, f"qualified dependency drift: {name}"
    return dict(id=ID, ready=True, no_native_build=True, after_existing_bookings=True,
                controller="native-controller-complete-post-wal-queue",
                input_sha256=metrics.INPUT, cpus=[0, 1], postgres_cap=96, runner_topology=[32, 32],
                mesh_elf=factorial.ELF, mesh_sql=factorial.SQL_SHA, river_elf=receipt["candidate_elf"],
                stock_river_elf=receipt["stock_elf"], warm_tls_backends=67,
                offline_check_command=command("check"), replay_command=command("replay"),
                commands={stage: command("dispatch", stage) for stage in STAGES},
                schedule={stage: trials for stage, (trials, _) in STAGES.items()},
                native_window_minutes={stage: window for stage, (_, window) in STAGES.items()},
                evidence_destination=str(OUT.relative_to(ROOT)),
                local_sha256={str(path.relative_to(ROOT)): run.digest(path) for path in sorted(set(files))})


def registered():
    assert (QUEUE / f"{ID}.json").read_bytes() == PACKET.read_bytes(), "controller registration required"


def last_event():
    return re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+)(?: ([^\s,]+))?(?: ([^\s,]+))?",
                      (QUEUE / "STATUS.md").read_text(), re.M)[-1]


def active(stage):
    registered()
    kind, name, start, end = last_event()
    assert (kind, name) == ("SETUP" if stage == "setup" else "TIMING", f"{ID}-{stage}")
    assert datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
    return dict(id=name, start=start, end=end, request_sha256=run.digest(PACKET))


def install(receipt, cfg):
    binary = binary_path(receipt)
    pins = cfg["lanes"]["river-2x32"]["inventory"]["engines"]["go"]["verified_files"]
    checksum_input = "\n".join(f"{digest}  {path}" for path, digest in sorted(pins.items()))
    script = (HOST_IDLE + (HERE / "install.sh").read_text().replace("@PINNED_FILES@", checksum_input))
    with build.BINARY.open("rb") as data, (OUT / "install.log").open("xb") as log:
        result = run.ssh(["bash", "-c", script, "--", binary, receipt["compressed_sha256"],
                          receipt["candidate_elf"], *cfg["host"], cfg["clock_ticks"]],
                         stdin=data, stdout=log, stderr=log, timeout=75)
    result.check_returncode()


def dispatch(stage, receipt, cfg):
    grant = active(stage)
    OUT.mkdir(parents=True, exist_ok=True)
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        active(stage)
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
        nonce = time.time_ns()
        build.dump(OUT / f"grant-{stage}-{nonce}.json", grant)
        if stage == "setup":
            install(receipt, cfg)
        for lane in LANES:
            path = OUT / "staging" / f"{stage}-{nonce}" / lane
            path.mkdir(parents=True)
            inventory = path / "inventory.json"
            build.dump(inventory, cfg["lanes"][lane]["inventory"])
            files = (*factorial.FILES, SUITE / "worker.py", inventory) if lane == "combined" else (
                SUITE / "worker.py", inventory)
            with (path / "stage.log").open("xb") as log:
                for source in files:
                    active(stage)
                    run.stage(source, remote(lane), log)
        for trial in STAGES[stage][0]:
            path = OUT / "blocks" / f"{run.block_prefix(trial)}.{time.time_ns()}"
            path.mkdir(parents=True)
            build.dump(path / "order.json", trial)
            for phase, lane, assigned, folder in factorial.grouped.matrix.paths(path, cfg, trial):
                end = datetime.fromisoformat(active(stage)["end"])
                assert (end - datetime.now(timezone.utc)).total_seconds() >= 125, "grant time budget exhausted"
                folder.parent.mkdir()
                run.dispatch_arm(folder.parent, remote(lane), trial, phase, assigned["engine"])
                score_arm(folder, phase, lane, cfg)
            build.dump(path / "reconciled.json", replay_block(path, trial, cfg))


def intake(stage, seal, request, cfg):
    assert seal == run.digest(PACKET)
    registered()
    if release_path(stage).exists():
        return build.read(release_path(stage))["exit_status"]
    if stage != "setup":
        require_phase("setup", cfg)
    if stage == "rest":
        require_phase("pilot", cfg)
    assert last_event()[0] == "RELEASED", "earlier native work still owns the controller"
    owner = runpy.run_path(str(QUEUE.parent / "completion-writer-r2.py"))
    trials, minutes = STAGES[stage]
    groups = [trial["group"] for trial in trials]
    identifier = f"{ID}-{stage}"
    control = OUT / "controller"
    control.mkdir(parents=True, exist_ok=True)
    if stage == "setup":
        owner["probe"]()
        start = datetime.now(timezone.utc)
        end = start + timedelta(minutes=minutes)
        owner["note"](f"SETUP {identifier} {start:%Y-%m-%dT%H:%M:%SZ} {end:%Y-%m-%dT%H:%M:%SZ}\n\n"
                      f"Sealed request {seal}.")
        status = 1
        try:
            with (control / "setup.log").open("xb") as log:
                status = subprocess.run(shlex.split(request["commands"][stage]), cwd=ROOT,
                                        stdout=log, stderr=log).returncode
        finally:
            owner["probe"]()
            now = run.utc_now()
            build.dump(release_path(stage), dict(id=identifier, groups=groups, exit_status=status,
                                                 released_at=now, started=start.isoformat()))
            owner["note"](f"RELEASED {identifier} {now}, exit {status}. Evidence: {OUT}.")
        return status
    key = f"{identifier}-groups-{groups[0]}-{groups[-1]}"
    saved = QUEUE / f"{key}.released.json"
    if not saved.exists():
        owner["measure"](dict(id=ID, timing_grant_id=identifier, dispatch=request["commands"][stage],
                              timing_window_minutes=minutes, evidence_destination=str(OUT),
                              controller_receipt_dir=str(control), command_attempts=1), "dispatch", groups)
    release_path(stage).write_bytes(saved.read_bytes())
    return build.read(release_path(stage))["exit_status"]


def main(args):
    receipt = build.check()
    cfg = config(receipt)
    if args == ["seal"]:
        assert not PACKET.exists() and not HISTORY.exists()
        build.dump(HISTORY, history())
        build.dump(PACKET, packet(receipt))
        print(run.digest(PACKET))
        return 0
    request = packet(receipt)
    assert request == build.read(PACKET)
    if args == ["check"]:
        assert build.read(HISTORY) == json.loads(json.dumps(history()))
        print(f"{ID} verified {run.digest(PACKET)}")
        return 0
    if args == ["replay"]:
        groups, dispositions = {}, {}
        for stage in STAGES:
            path = release_path(stage)
            if not path.exists():
                dispositions[stage] = "no release receipt"
                continue
            dispositions[stage] = build.read(path)
            if dispositions[stage]["exit_status"] == 0:
                groups.update(require_phase(stage, cfg))
        measured = {index: row["measured"] for index, row in groups.items() if "measured" in row}
        print(json.dumps(dict(dispositions=dispositions, groups=groups,
                              paired=metrics.comparisons(measured) if measured else None),
                         sort_keys=True, indent=2, allow_nan=False))
        return 0
    if len(args) == 2 and args[0] == "dispatch" and args[1] in STAGES:
        dispatch(args[1], receipt, cfg)
        return 0
    if len(args) == 3 and args[0] == "intake" and args[1] in STAGES:
        return intake(args[1], args[2], request, cfg)
    raise SystemExit("Use check, replay, or intake {setup,pilot,rest} SEAL.")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
