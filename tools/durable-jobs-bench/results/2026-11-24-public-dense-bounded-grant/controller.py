"""Seal, dispatch under the existing controller, or replay the dense grant packet."""

from datetime import datetime, timezone
import gzip
from hashlib import sha256
from importlib import import_module
from itertools import permutations
import json
from pathlib import Path
import re
import runpy
import shlex
from statistics import median
import subprocess
import sys
import tarfile

from diagnostic import CANDIDATE, STOCK, assess, body, digest as sql_digest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
SUITE = HERE.parents[1]
ID = HERE.name
PEER_ID = ID + "-peers"
PACKET = HERE / "controller-packet.json"
OUT = HERE / "evidence/native"
STATE = ROOT / "target/linode-jobs-stepchange"
QUEUE = STATE / "requests"
PREDECESSOR = HERE.parent / "2026-11-24-public-dense-set-claim"
MATRIX = HERE.parent / "2026-11-23-bulk-enqueue-v2-short/replay/interleave-v2.py"
sys.path[:0] = [str(SUITE), str(MATRIX.parent), str(PREDECESSOR)]
from audit import ARCHIVE, ELF, INPUT, MANIFEST, REVISION, SOURCE, digest, lifecycle, member, read

matrix = import_module(MATRIX.stem)
run = matrix.run
FILES = ("launch.sh", "adapter.py", "diagnostic.py", "stock.sql")
MODES = {"qualify": "qualify", "control-a": "control", "control-b": "control", "candidate": "candidate"}
CONTROL = ("control-a", "control-b", "candidate")
PEERS = ("candidate", "river", "oban")
HOST = ["90ec1c24-08c8-4bc1-aa28-79b6db1bd13a", "6.8.0-149-generic", "8:0"]
BASE = ARCHIVE / "dense-equal/controller-evidence/series.json"
SETUP = SOURCE / "evidence/native/setup/trusted-jobs-product-623c46b92cc5-closure-retry"
BATCH = PREDECESSOR / "evidence/batch_000000.json.gz"
OLD_RESULTS = PREDECESSOR / "evidence/native/final-replay.json"


def block(group, order, phases=("measured",)):
    return dict(group=group, profile="noop", phases=list(phases), order=list(order))


TRIALS = [block(0, ["qualify"])] + [
    block(i, order) for i, order in enumerate(permutations(CONTROL), 1)
]
PEER_TRIALS = [
    block(i, order, ("warmup", "measured")) for i, order in enumerate(permutations(PEERS), 7)
]
BLOCKS = TRIALS + PEER_TRIALS


def config():
    cfg = read(BASE)["config"]
    base = cfg["lanes"]
    cfg.update(lanes={}, description="Public dense grant, identical 2x32 product ELF with one SQL predicate")
    for label, mode in MODES.items():
        lane = json.loads(json.dumps(base["product"]))
        spec = lane["inventory"]["engines"]["mesh"]
        assert spec["artifact_sha256"] == MANIFEST
        assert f"PRODUCT_ELF_SHA256={ELF}" in spec["argv"] and "PRODUCT_TOPOLOGY=two-runner" in spec["argv"]
        old = spec["argv"][-2]
        assert old.endswith("/replay-closure-retry/native-run") and spec["argv"][-1] == "mesh"
        stage = run.REMOTE / f"{ID}-{label}"
        spec["argv"][-2:-1] = ["/bin/bash", str(stage / "launch.sh"), old, mode]
        spec["verified_files"].update({str(stage / name): digest(HERE / name) for name in FILES})
        cfg["lanes"][label] = lane
    cfg["lanes"].update({key: base[key] for key in ("river", "oban")})
    return cfg


def command(action):
    return shlex.join(["python3", "-B", str((HERE / "controller.py").relative_to(ROOT)), action])


def packet():
    paths = [*(HERE / name for name in (*FILES, "controller.py", "README.md")),
             MATRIX, BASE, BATCH, OLD_RESULTS, PREDECESSOR / "audit.py",
             SETUP / "qualification.json", SETUP / "controller-release.json", SETUP / "raw.tar.gz",
             *(SUITE / name for name in ("run.py", "verify.py", "worker.py"))]
    return dict(id=ID, ready=True, state="ready-for-sole-controller-intake",
                controller="linode-native-stepchange-controller-v1", no_build=True, cpus=[0, 1],
                native_host=HOST, revision=REVISION, pg_version="17.11",
                elf_sha256=ELF, manifest_sha256=MANIFEST, input_sha256=INPUT,
                timing_grant_id=ID, peer_grant_id=PEER_ID,
                timing_window_minutes=16, peer_window_minutes=20,
                offline_check=command("check"), timing_command=command("timing"),
                peer_command=command("peers"), replay_command=command("replay"),
                controller_receipt_dir=str(OUT / "controller"), evidence_destination=str(OUT),
                resource_bounds=dict(pg_cap=96, app_tls_sessions=32, one_arm_at_a_time=True,
                                     qualification_arms=1, primary_arms=18, peer_arms=36),
                schedule=TRIALS, peer_schedule=PEER_TRIALS,
                local_sha256={str(path.relative_to(ROOT)): digest(path) for path in paths})


def check():
    assert read(PACKET) == packet(), "sealed packet or dependency changed"
    cfg = config()
    assert cfg["host"] == HOST and cfg["cpus"] == [0, 1] and cfg["profiles"] == ["noop"]
    matrix.validate(cfg)
    source = subprocess.check_output(["git", "show", f"{REVISION}:compiler/mesh-rt/src/jobs/schema.sql"],
                                     cwd=ROOT, text=True)
    assert source.count(STOCK) == 1 and CANDIDATE.replace(
        "\n            AND j.id = ANY (ARRAY(SELECT k.id FROM sized k))", "") == STOCK
    subprocess.run(["bash", "-n", HERE / "launch.sh"], check=True)
    for path in HERE.glob("*.py"):
        compile(path.read_bytes(), str(path), "exec")
    expected = sha256(gzip.decompress(BATCH.read_bytes())).hexdigest()
    assert sha256(expected.encode()).hexdigest() == INPUT
    proof = read(SETUP / "qualification.json")
    assert proof["executable_sha256"] == ELF and proof["manifest_sha256"] == MANIFEST
    assert digest(SETUP / "raw.tar.gz") == proof["raw_sha256"]
    assert proof["files"].items() <= cfg["lanes"]["qualify"]["inventory"]["engines"]["mesh"]["verified_files"].items()
    return cfg


def grant(key):
    assert (QUEUE / f"{key}.json").read_bytes() == PACKET.read_bytes()
    status = (QUEUE / "STATUS.md").read_text()
    active, name, start, end = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+)(?: ([^\s,]+))?(?: ([^\s,]+))?",
                                       status, re.M)[-1]
    assert (active, name) == ("TIMING", key)
    assert datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
    return dict(grant=key, start=start, end=end, request_sha256=digest(PACKET))


def score(folder, phase, label, trial, cfg):
    lane = cfg["lanes"][label]
    old = matrix.PROFILES["noop"]
    matrix.PROFILES["noop"] = (2, *old[1:])
    try:
        result = run.score_arm(folder, trial, phase, lane["engine"], lane["inventory"])
    finally:
        matrix.PROFILES["noop"] = old
    assert [result["host_boot_id"], result["host_kernel"], result["pg_mount"]["maj:min"]] == cfg["host"]
    result.update(submit_ms=(result["last_settlement_us"] - result["first_submit_us"]) / 1000,
                  drain_ms=(result["last_settlement_us"] - result["first_commit_us"]) / 1000,
                  raw_sha256=digest(folder / "raw.tar.gz"))
    if label not in MODES:
        return result
    with tarfile.open(folder / "raw.tar.gz") as archive:
        ledger, writer, policy = (member(archive, name) for name in ("ledger", "producer-writer", "queue-policy"))
        lifecycle(ledger, writer, policy)
        assert (ledger["runners"], ledger["slots"]) == (2, 64) and ledger["peak_connections"] <= 41
        assert member(archive, "build")["files"]["mesh"] == ELF
        assert len(member(archive, "process-identity")) == 3
        tls = member(archive, "postgres-tls-sessions")
        assert all(row["tls"] for row in tls) and sum(row["role"] == "compiled_caller" for row in tls) == 32
        raw = member(archive, "raw")
        assert raw["statement_counter_deallocations"] == 0
        statements = policy["queue_statements"]
        top, = [s for s in statements if "FROM mesh_jobs.claim_batch_set_v1(" in s["query"]]
        assert top["calls"] == policy["admitted_queries"] and top["rows"] >= 1024
        assert not any("FROM mesh_jobs.claim" in s["query"] and s != top for s in statements)
        reports = [json.loads(archive.extractfile(f"adapter/runner-{i}.log").read().splitlines()[-1])
                   for i in range(2)]
        assert all(r["last_error"] is None and r["drained"] and r["pending_claims"] == 0
                   and r["claimed"] == r["finished"] for r in reports)
        assert sum(r["claimed"] for r in reports) == 1024
        assert [r["wakeups"] for r in reports] == raw["wakeups"]
        masks = member(archive, "process-affinity")
        assert len(masks) == 3 and all(mask == cfg["cpus"] for mask in masks.values())
        overlay = member(archive, "grant-overlay")
        mode = MODES[label]
        assert overlay["mode"] == mode
        assert overlay["body_sha256"] == sql_digest(body(CANDIDATE if mode == "candidate" else STOCK))
        assert overlay["baseline_body_sha256"] == sql_digest(body(STOCK))
        counters = member(archive, "grant-counters")
        assert counters["plan_cache_mode"] == "auto" and counters["auto_explain"] in (None, "-1")
        assert counters["jobs"]["n_tup_ins"] == 1024 and counters["jobs"]["n_tup_upd"] == 2048
        postgres = member(archive, "postgres-settings")
        assert postgres["version"] == "17.11" and postgres["max_connections"] == 96
        assert all(postgres[key] for key in ("tls", "fsync", "full_page_writes", "synchronous_commit",
                                             "track_commit_timestamp"))
        if mode == "qualify":
            proof = member(archive, "grant-plans")
            result["qualification"] = assess(proof)
            assert proof["gate"] == result["qualification"] and result["qualification"]["passed"]
        result.update(jobs_fetch=counters["jobs"]["idx_tup_fetch"], jobs_indexes=counters["indexes"],
                      hints=sum(r["batch_contention_hints"] for r in reports), claim=top)
    return result


def replay_block(path, cfg, trial):
    assert read(path / "order.json") == trial
    phases = {phase: {} for phase in trial["phases"]}
    for phase, label, folder in matrix.arms(path, cfg, trial):
        phases[phase][label] = score(folder, phase, label, trial, cfg)
    return phases


def timing_gate(groups):
    if not all(groups.get(trial["group"]) for trial in TRIALS):
        return dict(passed=False, reason="primary evidence incomplete")
    diffs = []
    for trial in TRIALS[1:]:
        g = trial["group"]
        rows = groups[g][1]["measured"]
        a, b, c = [rows[name] for name in CONTROL]
        center = (a["submit_ms"] + b["submit_ms"]) / 2
        spread = abs(a["submit_ms"] - b["submit_ms"])
        fetch = (a["jobs_fetch"] + b["jobs_fetch"]) / 2
        diffs.append(dict(group=g, control_ms=center, candidate_ms=c["submit_ms"],
                          control_spread_ms=spread, saved_ms=center - c["submit_ms"],
                          faster_than_both=c["submit_ms"] < min(a["submit_ms"], b["submit_ms"]),
                          control_fetch=fetch, candidate_fetch=c["jobs_fetch"],
                          candidate_drain_ms=c["drain_ms"], control_drain_ms=(a["drain_ms"]+b["drain_ms"])/2))
    saved = median(d["saved_ms"] for d in diffs)
    center = median(d["control_ms"] for d in diffs)
    spread = median(d["control_spread_ms"] for d in diffs)
    total_fetch = sum(d["control_fetch"] for d in diffs)
    checks = dict(
        wall=saved >= max(75, 0.10 * center, 2 * spread),
        paired_wins=sum(d["faster_than_both"] for d in diffs) >= 5,
        fetch=total_fetch > 0 and sum(d["candidate_fetch"] for d in diffs) <= 0.75 * total_fetch,
    )
    return dict(passed=all(checks.values()), checks=checks, groups=diffs)


def peer_gate(groups):
    if not all(groups.get(trial["group"]) for trial in PEER_TRIALS):
        return dict(passed=False, reason="peer evidence incomplete")
    rows = [groups[trial["group"]][1]["measured"] for trial in PEER_TRIALS]
    ratios = {peer: [row["candidate"]["submit_ms"] / row[peer]["submit_ms"] for row in rows]
              for peer in ("river", "oban")}
    return dict(passed=all(max(values) < 1 and median(values) <= 0.8 for values in ratios.values()),
                ratios=ratios)


def replay(cfg):
    groups, failed, repeats = {}, {}, {}
    for trial in BLOCKS:
        group = trial["group"]
        groups[group] = None
        for path in matrix.attempt_paths(OUT, trial):
            try:
                scored = replay_block(path, cfg, trial)
            except matrix.ATTEMPT_ERRORS as error:
                failed[path.name] = f"{type(error).__name__}: {error}"
            else:
                if groups[group] is None:
                    groups[group] = (path.name, scored)
                else:
                    repeats[path.name] = scored
    return dict(groups=groups, failures=failed, additional_complete_attempts=repeats,
                next_peer_gate=timing_gate(groups), peer_performance_gate=peer_gate(groups))


def dispatch(cfg, peers):
    key, trials = (PEER_ID, PEER_TRIALS) if peers else (ID, TRIALS)
    active = grant(key)
    if peers:
        assert replay(cfg)["next_peer_gate"]["passed"], "primary plan and timing gate did not pass"
    OUT.mkdir(parents=True, exist_ok=True)
    run.write_json(OUT / f"grant-{datetime.now(timezone.utc):%Y%m%dT%H%M%S%f}.json", active)
    series = OUT / "series.json"
    frozen = dict(config=cfg, plan=BLOCKS,
                  drivers={p.name: digest(p) for p in (MATRIX, SUITE/"run.py", SUITE/"verify.py", SUITE/"worker.py")},
                  remote={label: f"{ID}-{label}" for label in cfg["lanes"]})
    if not series.exists():
        run.write_json(series, frozen)
    assert read(series) == frozen
    original_stage, original_arm, original_block = run.stage, run.dispatch_arm, matrix.dispatch_block

    def stage(path, remote, log):
        grant(key)
        if path.name == "inventory.json" and remote.name.removeprefix(ID + "-") in MODES:
            for name in FILES:
                original_stage(HERE / name, remote, log)
        original_stage(path, remote, log)

    def arm(*args):
        grant(key)
        return original_arm(*args)

    stopped = False

    def assigned(directory, manifest, trial):
        nonlocal stopped
        if stopped:
            return False
        grant(key)
        passed = original_block(directory, manifest, trial)
        stopped = trial["group"] == 0 and not passed
        return passed

    run.stage, run.dispatch_arm, matrix.dispatch_block = stage, arm, assigned
    matrix.plan = lambda *_: iter(BLOCKS)
    matrix.replay = replay_block
    return matrix.dispatch(OUT, cfg, [trial["group"] for trial in trials])


def intake(cfg, pin, peers):
    assert digest(PACKET) == pin, "intake packet does not match the announced SHA-256"
    key, start, end = (PEER_ID, 7, 12) if peers else (ID, 0, 6)
    if peers:
        assert replay(cfg)["next_peer_gate"]["passed"], "primary plan and timing gate did not pass"
    release = QUEUE / f"{key}-groups-{start}-{end}.released.json"
    if release.exists():
        return read(release)["exit_status"]
    request = QUEUE / f"{key}.json"
    try:
        with request.open("xb") as stream:
            stream.write(PACKET.read_bytes())
    except FileExistsError:
        assert request.read_bytes() == PACKET.read_bytes()
    sealed = read(PACKET)
    sealed.update(timing_grant_id=key, command_attempts=1,
                  timing_window_minutes=sealed["peer_window_minutes" if peers else "timing_window_minutes"])
    ops = runpy.run_path(str(STATE / "completion-writer-r2.py"))
    return ops["measure"](sealed, "peer_command" if peers else "timing_command", range(start, end + 1))


def main(args):
    if args == ["seal"]:
        assert not PACKET.exists()
        run.write_json(PACKET, packet())
        return 0
    cfg = check()
    matrix.ATTEMPT_ERRORS += (AssertionError, StopIteration)
    if args == ["check"]:
        print(f"{ID}: SQL overlay, native inventory, both schedules, and receipt pins verified")
        return 0
    if args == ["replay"]:
        print(json.dumps(replay(cfg), indent=2, sort_keys=True, allow_nan=False))
        return 0
    if args in (["timing"], ["peers"]):
        return dispatch(cfg, args[0] == "peers")
    if len(args) == 2 and args[0] in {"intake", "intake-peers"}:
        return intake(cfg, args[1], args[0] == "intake-peers")
    raise SystemExit("usage: controller.py check|replay|timing|peers|intake SHA256|intake-peers SHA256")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
