"""Seal and replay the short transfer. Native dispatch belongs to the sole controller."""
import argparse
from copy import deepcopy
from datetime import datetime, timezone
import fcntl
import importlib.util
import json
from pathlib import Path
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
ID = HERE.name
PEERS_ID = ID + "-peers"
OUT = HERE / "evidence/native"
PACKET = HERE / "controller-packet.json"
PRECHECK = HERE / "offline.json"
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
DENSE = RESULTS / "2026-11-25-dense-fixed8-logical-factorial"
GROUPED = RESULTS / "2026-11-25-dense-grouped-refill"
LOGICAL = RESULTS / "2026-11-25-dense-logical-grant-key-native"
SHORT = RESULTS / "2026-11-24-short16-river-readiness"
PUBLIC = RESULTS / "2026-11-24-trusted-jobs-product-composition"
MODEL = RESULTS / "2026-11-25-dense-factorial-interaction"
SOURCE = ROOT / "target/dense-grouped-refill-packet/source.tar.gz"
FROZEN = ROOT / "target/dense-public-hot-readiness/ordinary-v2"
NATIVE_FROZEN = Path("/var/lib/durable-jobs-bench/native/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2")
NEW_FILES = tuple(HERE / name for name in ("launch.sh", "render_short.py", "short_arm.py"))
SQL_FILES = tuple(LOGICAL / name for name in ("contract.py", "stock.sql", "candidate.sql"))
FILES = NEW_FILES + SQL_FILES
INPUT = "ac0bcbf2465ea6d6807e8bfcdcfc3b1dacfa18b775e3f4c074257d66bfbba9d1"

sys.path[:0] = [str(SUITE), str(LOGICAL), str(GROUPED)]
import run
from interleave import HOST_IDLE
from render_short import render
from scoring import check_terminal, gate, measure, occupancy, peer_pairs


def load(label, file):
    spec = importlib.util.spec_from_file_location(label, file)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


grouped = load("grouped_refill_pilot", GROUPED / "pilot.py")
transfer = grouped.public
HOST = ["90ec1c24-08c8-4bc1-aa28-79b6db1bd13a", "6.8.0-149-generic", "8:0"]
ELF = "d56d0e6112a94ff601bebf6f777061068ed2fe0f2f2da586963dd6e1326a220a"
GRANT_SHA = "d388baa3b67c07df1a2498cd1fdc60bf7d0ce11f4b42d950f7b0a64a551c0f45"
readiness = load("short_readiness_score", SHORT / "score.py")
MODES = {"proof-immediate": "", "proof-fixed": "fixed8ms", "proof-refill": "refill8ms",
         "immediate-a": "", "immediate-b": "", "fixed": "fixed8ms", "refill": "refill8ms"}
PEERS = {"river": "go", "river-warm": "go", "oban": "oban", "apalis": "rust"}
ORDER = ("immediate-a", "fixed", "refill", "immediate-b")
CYCLE = ("refill", *PEERS)


def block(group, order, phases=("warmup", "measured")):
    return dict(group=group, order=list(order), phases=list(phases), profile="short")


PILOT = [block(0, ("proof-immediate", "proof-fixed", "proof-refill"), ("diagnostic",)),
         block(1, ORDER), block(2, ORDER[::-1])]
PEER_PLAN = [block(index + 3, CYCLE[index:] + CYCLE[:index]) for index in range(len(CYCLE))]


def unique(paths):
    item, = sorted(paths)
    return item


def winner_inventory():
    # Use the measured peer series staging, which is unique and retains the winning argv and pins.
    return unique((DENSE / "evidence/native-retry/staging").glob("*-peers.*/combined/inventory.json"))


PEER_RELEASES = tuple(QUEUE / f"{SHORT.name}-groups-{span}.released.json" for span in ("1-2", "3-6"))


def ready_inventory(lane):
    return SHORT / "evidence/timing" / lane / "inventory.json"


def peer_history(lane):
    first = unique((SHORT / "evidence/timing/blocks").glob("01-short.*"))
    return first / f"measured-{lane}" / f"measured-{PEERS[lane]}"


def release_path(key, groups):
    return QUEUE / f"{key}-groups-{groups[0]}-{groups[-1]}.released.json"


def remote(lane):
    return run.REMOTE / f"{ID}-{lane}"


def config():
    binding = run.read_json(GROUPED / "evidence/native-setup/binding.json")
    series = run.read_json(GROUPED / "evidence/native-pilot/series.json")["config"]
    winner = run.read_json(winner_inventory())
    assert winner["cpus"] == series["cpus"] == [0, 1] and series["host"] == HOST
    assert binding["build"]["files"]["mesh"] == ELF
    assert binding["manifest_sha256"] == winner["engines"]["mesh"]["artifact_sha256"]
    candidate = (LOGICAL / "candidate.sql").read_text()
    assert run.digest(LOGICAL / "candidate.sql") == GRANT_SHA
    lanes = {}
    for lane in MODES:
        inventory = deepcopy(winner)
        spec = inventory["engines"]["mesh"]
        argv, pins = spec["argv"], spec["verified_files"]
        assert argv[-2:] == ["combined", "mesh"] and argv[1] == "JOBS_GROUPED_WRITER=fixed8ms"
        assert argv[4] == "PRODUCT_TOPOLOGY=two-runner"
        argv[1], argv[4] = f"JOBS_GROUPED_WRITER={MODES[lane]}", "PRODUCT_TOPOLOGY=stock"
        prior_launcher = Path(argv[-4])
        assert pins.pop(str(prior_launcher)) == run.digest(DENSE / "launch.sh")
        for name in ("render.py", "factorial_arm.py", "contract.py", "stock.sql", "candidate.sql", "worker.py"):
            pins.pop(str(prior_launcher.parent / name))
        argv[-4], argv[-2] = str(remote(lane) / "launch.sh"), lane
        pins.update({str(remote(lane) / path.name): run.digest(path) for path in FILES})
        lanes[lane] = dict(engine="mesh", inventory=inventory, mode=MODES[lane])
    for lane, engine in PEERS.items():
        lanes[lane] = dict(engine=engine, inventory=run.read_json(ready_inventory(lane)))
    for lane, item in lanes.items():
        item["inventory"]["engines"][item["engine"]]["verified_files"][
            str(remote(lane) / "worker.py")] = run.digest(SUITE / "worker.py")
        run.check_inventory(item["inventory"])
    return dict(host=series["host"], clock_ticks=series["clock_ticks"], product=binding, candidate=candidate, lanes=lanes,
                grant_metadata=dict(owner="parity_migrator", security_definer=False, worker_execute=True,
                                    producer_execute=False, app_execute=False,
                                    configuration=["search_path=pg_catalog, pg_temp", "lock_timeout=100ms"],
                                    acl=["parity_migrator=X/parity_migrator", "parity_subject=X/parity_migrator"]))


def qualified_peers(cfg):
    for release in PEER_RELEASES:
        assert run.read_json(release)["exit_status"] == 0
    result = {}
    for lane, engine in PEERS.items():
        inventory = run.read_json(ready_inventory(lane))
        folder = peer_history(lane)
        old_cfg = dict(cfg, lanes={lane: dict(inventory=inventory, engine=engine)})
        score = measure(folder, "measured", lane, old_cfg, run, transfer, readiness)
        expected = 18 if lane in {"river", "river-warm"} else 19
        assert score["connections"]["peak_during_load"] == expected
        assert score["connections"]["last_idle"] == (4 if lane == "river" else expected)
        result[lane] = dict(inventory_sha256=run.digest(ready_inventory(lane)), raw_sha256=score["raw_sha256"],
                            completed=score["completed"], last_idle=score["connections"]["last_idle"],
                            load_peak=score["connections"]["peak_during_load"])
    return result


def source_check(cfg):
    setup = GROUPED / "evidence/native-setup"
    binding = grouped.setup(SOURCE, setup / "controller-release.json", setup / "raw.tar.gz")
    assert binding == cfg["product"]
    fixture = f"source/{PUBLIC.relative_to(ROOT)}/fixture/main.mpl"
    launcher = f"source/{PUBLIC.relative_to(ROOT)}/replay/native-run"
    adapter = f"source/{PUBLIC.relative_to(ROOT)}/replay/backend.py"
    release = run.read_json(release_path(DENSE.name + "-retry-peers", [3, 4, 5, 6, 7]))
    assert release["exit_status"] == 0 and release["groups"] == [3, 4, 5, 6, 7]
    layout = run.read_json(winner_inventory())["engines"]["mesh"]
    pins = layout["verified_files"]
    source_root = Path(layout["argv"][-3]).parents[5]
    root = FROZEN.resolve()
    frozen_pins = {Path(path).relative_to(NATIVE_FROZEN): value for path, value in pins.items()
                   if Path(path).is_relative_to(NATIVE_FROZEN)}
    for path, pin in frozen_pins.items():
        assert run.digest(root / path) == pin, path
    trial = (root / "proof/jobs-parity/linux-regression/trial.py").read_text()
    assert f'"short": (1, 16, 8, 20, 2, "{INPUT}")' in trial
    assert 'json.loads(path.read_text()) if self.engine == "mesh" else str(path)' in trial
    assert 'request({"verb": "enqueue", "batch": batch})' in trial and 'reply() == "SESSION_READY"' in trial
    frozen_arm = (root / "tools/durable-jobs-bench/native/arm.py").read_text()
    assert 'JOBS_PARITY_MESH_BATCH="1", JOBS_PARITY_MESH_LISTEN="1"' in frozen_arm
    with tarfile.open(SOURCE) as archive:
        get = lambda name: archive.extractfile(name).read().decode()
        main = get(fixture)
        for fragment in ('action == "session" do return session()', 'let line = IO.read_line()?',
                         "count - index > 128", "next_bytes > 1_048_576",
                         "Jobs.enqueue_in(conn, namespace(), batch)", "Json.array_length(receipts)? != size",
                         "Pool.open(Env.get(\"MESH_EFFECT_APP_URL\", \"\"), lanes, lanes, 5000)",
                         'Env.get_int("JOBS_PARITY_SLOTS", 0)', 'Jobs.join(runner)',
                         '"fixed8ms" ->', '"refill8ms" ->'):
            assert fragment in main, fragment
        assert '"product-d2": ("success_cohort", True)' in get(adapter)
        assert 'mesh_pool_hot_checkout=1' in get(adapter)
        rendered = render(get(launcher), source_root, "proof-fixed")
        assert 'runuser -u nobody' in rendered and rendered.count('"$here/short_arm.py" proof-fixed') == 2
        subprocess.run(["bash", "-n"], input=rendered, text=True, check=True)
        runtime = get("source/compiler/mesh-rt/src/jobs/grouped_writer.rs")
        assert "refill_due && self.policy == GroupedPolicy::Refill8ms" in runtime
        assert "take(MAX_COMPLETE_COHORT)" in runtime and "ready_at(&state, attempt)" in runtime
        jobs = "source/compiler/mesh-rt/src/jobs/"
        runner = get(jobs + "runner.rs")
        store = get(jobs + "store.rs")
        assert "Ok(WorkerPoll::Exited(result))" in runner and "Execution::acknowledging_at(reply, duration_ms, now)" in runner
        assert "writer.flush(runner, &mut io, &attempts, saturated_due)" in runner
        grant = store.split("fn checked_claim<T>", 1)[1].split("fn decode_checked_batch", 1)[0]
        assert grant.index(".claim_rows(sql, params, limit)") < grant.index("JobsClaimCommand::Commit")
        assert grant.index("JobsClaimCommand::Commit") < grant.index("Ok(Some(batch))", grant.index("JobsClaimCommand::Commit"))
    replay = subprocess.check_output([sys.executable, "-B", "short_transfer.py"], cwd=MODEL)
    assert replay == (MODEL / "short-transfer.json").read_bytes()
    return dict(elf_sha256=ELF, manifest_sha256=binding["manifest_sha256"], candidate_sha256=GRANT_SHA,
                teardown_and_receipt_proofs=binding["proofs"],
                source_sha256=binding["source_sha256"], historical_replay_sha256=run.digest(MODEL / "short-transfer.json"),
                frozen_source_pins={str(key): pin for key, pin in frozen_pins.items()},
                transport="persistent compiled session, newline JSON stdin, inline batch, one parity_producer TLS session",
                jobs=128, slots=16, app_tls_sessions=8, postgres_connection_cap=20)


def replay_grouped_control():
    history = PUBLIC / "evidence/native/timing-closure-retry/short-smaller"
    block_path = unique((history / "blocks").glob("01-short.*"))
    folder = block_path / "measured-product-group/measured-mesh"
    with tarfile.open(folder / "raw.tar.gz") as archive:
        ledger = run.file_json(archive, "adapter/ledger.json")
        queue = run.file_json(archive, "adapter/queue-policy.json")
    jobs = ledger["jobs"]
    bounds = occupancy(jobs, ledger["transactions"][0]["committed_us"], max(job["settled_us"] for job in jobs))
    terminal = check_terminal(queue, jobs, None, "")
    return dict(raw_sha256=run.digest(folder / "raw.tar.gz"), occupancy=bounds, terminal=terminal)


def scores(path, trial, cfg):
    assert run.read_json(path / "order.json") == trial
    result, attempts = {}, set()
    for phase, lane, _, folder in grouped.matrix.paths(path, cfg, trial):
        attempt = run.read_json(folder / "request.json")["attempt"]
        assert attempt not in attempts
        attempts.add(attempt)
        result.setdefault(phase, {})[lane] = measure(folder, phase, lane, cfg, run, transfer, readiness)
    for rows in result.values():
        assert len({row["grant_sql"] for row in rows.values() if "grant_sql" in row}) == 1
    return result


def replay(cfg):
    groups, failures = {}, {}
    for trial in PILOT + PEER_PLAN:
        paths = list((OUT / "blocks").glob(f"{run.block_prefix(trial)}.*"))
        for path in sorted(paths):
            try:
                rows = scores(path, trial, cfg)
                assert len(paths) == 1, "duplicate controller block"
                assert run.read_json(path / "reconciled.json") == json.loads(json.dumps(rows))
            except (AssertionError, RuntimeError, OSError, ValueError, KeyError, StopIteration, tarfile.TarError) as error:
                failures[path.name] = f"{type(error).__name__}: {error}"
            else:
                groups[trial["group"]] = rows
    gates = {group: gate(groups[group]["measured"]) for group in (1, 2) if group in groups}
    measured = [groups[group] for group in range(3, 8) if group in groups]
    return dict(groups=groups, failures=failures, gates=gates,
                peer_eligible=not failures and set(PILOT_GROUPS) <= groups.keys() and all(x["passed"] for x in gates.values()),
                peer_pairs=peer_pairs(measured, PEERS) if len(measured) == 5 and not failures else None,
                raw_sha256={str(path.relative_to(OUT)): run.digest(path) for path in sorted(OUT.rglob("raw.tar.gz"))})


PILOT_GROUPS = [trial["group"] for trial in PILOT]


def clean_pilot(cfg):
    receipt = run.read_json(release_path(ID, PILOT_GROUPS))
    assert receipt["id"] == ID and receipt["groups"] == PILOT_GROUPS and receipt["exit_status"] == 0
    assert receipt["receipt"] == str(OUT) and receipt["released_at"]
    assert replay(cfg)["peer_eligible"], "both material-gain rotations must pass"


def active(key):
    assert (QUEUE / f"{key}.json").read_bytes() == PACKET.read_bytes()
    journal = (QUEUE / "STATUS.md").read_text()
    kind, name, start, end = re.findall(
        r"^(SETUP|TIMING|RELEASED|HOLD) (\S+)(?: ([^\s,]+))?(?: ([^\s,]+))?", journal, re.M)[-1]
    assert (kind, name) == ("TIMING", key)
    assert datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
    return dict(id=key, start=start, end=end, request_sha256=run.digest(PACKET))


def dispatch(cfg, peers):
    key, trials = (PEERS_ID, PEER_PLAN) if peers else (ID, PILOT)
    if peers:
        clean_pilot(cfg)
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
            files = (FILES if lane in MODES else ()) + (SUITE / "worker.py", inventory)
            with (folder / "stage.log").open("xb") as log:
                for source in files:
                    active(key)
                    run.stage(source, remote(lane), log)
        for trial in trials:
            path = OUT / "blocks" / f"{run.block_prefix(trial)}.{time.time_ns()}"
            path.mkdir(parents=True)
            run.write_json(path / "order.json", trial)
            for phase, lane, spec, folder in grouped.matrix.paths(path, cfg, trial):
                end = datetime.fromisoformat(active(key)["end"])
                assert (end - datetime.now(timezone.utc)).total_seconds() >= 125
                folder.parent.mkdir()
                run.dispatch_arm(folder.parent, remote(lane), trial, phase, spec["engine"])
                measure(folder, phase, lane, cfg, run, transfer, readiness)
            run.write_json(path / "reconciled.json", scores(path, trial, cfg))
    return 0


def intake(cfg, seal, peers):
    assert seal == run.digest(PACKET)
    if peers:
        clean_pilot(cfg)
    key, trials = (PEERS_ID, PEER_PLAN) if peers else (ID, PILOT)
    groups = [trial["group"] for trial in trials]
    assert (QUEUE / f"{key}.json").read_bytes() == PACKET.read_bytes(), "controller registration required"
    released = release_path(key, groups)
    if released.is_file():
        return run.read_json(released)["exit_status"]
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) ", (QUEUE / "STATUS.md").read_text(), re.M)
    assert events[-1] == "RELEASED", "earlier work still owns the native controller"
    request = run.read_json(PACKET)
    request.update(timing_grant_id=key, command_attempts=1,
                   timing_window_minutes=request["peer_window_minutes" if peers else "timing_window_minutes"])
    return runpy.run_path(str(QUEUE.parent / "completion-writer-r2.py"))["measure"](
        request, "peer_command" if peers else "timing_command", groups)


def command(action):
    return shlex.join(("python3", "-B", str((HERE / "controller.py").relative_to(ROOT)), action))


def packet(proof):
    inputs = {*FILES, *HERE.glob("*.py"), HERE / "README.md", PRECHECK, SOURCE,
              DENSE / "controller-packet-retry.json", DENSE / "launch.sh", GROUPED / "request.json", GROUPED / "package.py",
              GROUPED / "pilot.py", GROUPED / "candidate.patch", transfer.HERE / "transfer.py",
              transfer.PEER_ROOT / "matrix.py", transfer.PEER_ROOT / "qualification.py",
              SHORT / "score.py", MODEL / "short_transfer.py",
              MODEL / "short-transfer.json", winner_inventory(),
              *(ready_inventory(lane) for lane in PEERS), *PEER_RELEASES,
              release_path(DENSE.name + "-retry-peers", [3, 4, 5, 6, 7]),
              *(peer_history(lane) / "raw.tar.gz" for lane in PEERS),
              *(GROUPED / "evidence/native-setup" / name for name in ("binding.json", "raw.tar.gz", "controller-release.json")),
              GROUPED / "evidence/native-pilot/series.json",
              *(SUITE / name for name in ("run.py", "interleave.py", "worker.py", "verify.py")),
              *(FROZEN / name for name in proof["source"]["frozen_source_pins"])}
    return dict(id=ID, peers_id=PEERS_ID, ready=True, no_build=True, after_existing_bookings=True,
                controller="native-controller-complete-post-wal-queue", host=HOST, cpus=[0, 1],
                elf_sha256=ELF, logical_grant_sha256=GRANT_SHA, input_sha256=INPUT,
                profile=dict(jobs=128, runners=1, slots=16, app_tls_sessions=8, postgres_connection_cap=20),
                gate=dict(primary="refill", minimum_fraction=.05, duplicate_spread=True,
                          earlier_last_effect=True, max_cpu="higher duplicate on each CPU measure",
                          actual_refill_required=True),
                schedule=PILOT, peer_schedule=PEER_PLAN, timing_window_minutes=15, peer_window_minutes=23,
                offline_check=command("check"), timing_command=command("timing"), peer_command=command("peers"),
                replay_command=command("replay"), evidence_destination=str(OUT),
                controller_receipt_dir=str(OUT / "controller"),
                pins={str(path.relative_to(ROOT)): run.digest(path) for path in sorted(inputs)})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("seal", "check", "replay", "timing", "peers", "intake", "intake-peers"))
    parser.add_argument("sha", nargs="?")
    args = parser.parse_args()
    cfg = config()
    if args.action in {"seal", "check"}:
        proof = dict(source=source_check(cfg), qualified_peers=qualified_peers(cfg),
                     historical_grouped_control=replay_grouped_control())
        if args.action == "seal":
            assert not PACKET.exists() and not PRECHECK.exists()
            run.write_json(PRECHECK, proof)
            run.write_json(PACKET, packet(proof))
        else:
            assert run.read_json(PRECHECK) == json.loads(json.dumps(proof)) and run.read_json(PACKET) == packet(proof)
        print(json.dumps(proof, sort_keys=True))
        return 0
    for name, sha in run.read_json(PACKET)["pins"].items():
        assert run.digest(ROOT / name) == sha, f"sealed input changed: {name}"
    if args.action == "replay":
        print(json.dumps(replay(cfg), indent=2, sort_keys=True))
        return 0
    if args.action in ("timing", "peers"):
        return dispatch(cfg, args.action == "peers")
    return intake(cfg, args.sha, args.action == "intake-peers")


if __name__ == "__main__":
    raise SystemExit(main())
