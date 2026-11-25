"""Seal, replay and dispatch the stock 8×8 trial under the sole native owner."""

from copy import deepcopy
from datetime import datetime, timezone
import fcntl
from importlib import import_module
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
PACKET = HERE / "controller-packet.json"
OUT = HERE / "evidence/native"
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
PRODUCT = RESULTS / "2026-11-24-trusted-jobs-product-composition"
TRACE = RESULTS / "2026-11-24-public-dense-set-claim"
LIVE = RESULTS / "2026-11-25-dense-live-grant-plans"
KEY = RESULTS / "2026-11-25-dense-logical-grant-key-native"
MATRIX = RESULTS / "2026-11-23-bulk-enqueue-v2-short/replay"
SETUP = LIVE / "evidence/native/setup.json"
SERIES = PRODUCT / "evidence/native/timing-closure-retry/dense-stock/controller-evidence/series.json"
QUALIFIED = PRODUCT / "evidence/native/setup/trusted-jobs-product-623c46b92cc5-closure-retry/qualification.json"
CANDIDATE = KEY / "candidate.sql"
CLOSURE_BACKEND = PRODUCT / "closure-retry/backend.py"
MANIFEST_PATH = QUALIFIED.parent.with_name("trusted-jobs-product-623c46b92cc5") / "mesh.json"
KEY_SHA = "d388baa3b67c07df1a2498cd1fdc60bf7d0ce11f4b42d950f7b0a64a551c0f45"
FILES = (HERE/"launch.sh", HERE/"stage.py", HERE/"stock_arm.py", HERE/"roster8.py",
         *(KEY/f for f in ("candidate.sql", "stock.sql", "contract.py")),
         *(LIVE/f for f in ("observe.py", "settings.py", "plans.py", "launcher.py")),
         LIVE/"procfd-recovery/roster.py", SUITE/"worker.py")

sys.path[:0] = list(map(str, (HERE, TRACE, LIVE, LIVE/"procfd-recovery", KEY, SUITE, MATRIX)))
from audit import ELF, INPUT, MANIFEST, REVISION, member
from contract import STOCK, body, digest as sql_digest, validate
from interleave import HOST_IDLE, host_cpu
from score import DIAGNOSTIC, TRIPLE, diagnostic, pilot_gate, plan_gate, workload
from stage import launcher, observer

matrix = import_module("interleave-v2")
run = matrix.run
SCHEDULE = [dict(group=i, profile="noop", order=list(order), phases=["measured"])
            for i, order in enumerate((DIAGNOSTIC, TRIPLE, TRIPLE[::-1]))]


def remote(lane):
    return run.REMOTE / f"{ID}-{lane}"


def config():
    base = run.read_json(SERIES)["config"]
    stock = base["lanes"]["product"]
    qualification = run.read_json(QUALIFIED)
    base_inventory = stock["inventory"]["engines"]["mesh"]
    assert base["cpus"] == [0, 1] and base["clock_ticks"] == 100
    assert (base_inventory["version"], base_inventory["artifact_sha256"]) == (REVISION, MANIFEST)
    assert run.digest(MANIFEST_PATH) == MANIFEST
    assert qualification["executable_sha256"] == ELF and qualification["manifest_sha256"] == MANIFEST
    assert qualification["files"].items() <= base_inventory["verified_files"].items()
    assert run.digest(QUALIFIED.with_name("raw.tar.gz")) == qualification["raw_sha256"]
    argv = base_inventory["argv"]
    assert argv[2:7] == ["PRODUCT_COMPOSITION=product-bulk", "PRODUCT_TOPOLOGY=stock",
                        "PRODUCT_STORAGE=default", f"PRODUCT_ELF_SHA256={ELF}",
                        f"PRODUCT_MANIFEST_SHA256={MANIFEST}"]
    assert argv[-1] == "mesh"
    setup = run.read_json(SETUP)
    base["lanes"] = {}
    for lane in DIAGNOSTIC + TRIPLE:
        arm = deepcopy(stock)
        spec = arm["inventory"]["engines"]["mesh"]
        module = setup["remote"] if lane in DIAGNOSTIC else "-"
        spec["argv"] = [*argv[:-2], "/bin/bash", str(remote(lane)/"launch.sh"), argv[-2], lane, module, "mesh"]
        pins = spec["verified_files"]
        # Keep all native executable and immutable public source pins.
        pins.update({str(remote(lane)/path.name): run.digest(path) for path in FILES})
        if lane in DIAGNOSTIC:
            pins[module] = setup["sha256"]
        base["lanes"][lane] = arm
    matrix.validate(base)
    return base


def cmd(action):
    return shlex.join(["python3", "-B", str((HERE/"controller.py").relative_to(ROOT)), action])


def request(cfg):
    inputs = (*FILES, HERE/"controller.py", HERE/"score.py", HERE/"test_offline.py", HERE/"README.md",
              HERE/"history.json", HERE/"local-proof.sh", HERE/"stage_probe.py",
              HERE/"local-stage.txt", CLOSURE_BACKEND, MANIFEST_PATH, SERIES, QUALIFIED, SETUP, SETUP.with_name("setup-controller-release.json"),
              PRODUCT/"source.tar.gz", KEY/"proof.json", KEY/"evidence/native/final-replay.json",
              TRACE/"audit.py", TRACE/"evidence/native/final-replay.json", MATRIX/"interleave-v2.py",
              *(SUITE/f for f in ("run.py", "interleave.py", "verify.py")))
    return dict(id=ID, controller="native-controller-complete-post-wal-queue", ready=True, no_build=True,
                after_existing_bookings=True, native_host=cfg["host"], cpus=cfg["cpus"], candidate_sha256=KEY_SHA,
                input_sha256=INPUT, schedule=SCHEDULE, offline_check=cmd("check"), timing_command=cmd("timing"),
                replay_command=cmd("replay"), timing_window_minutes=14, evidence_destination=str(OUT),
                controller_receipt_dir=str(OUT/"controller"),
                local_sha256={str(path.relative_to(ROOT)): run.digest(path) for path in inputs})


def stage_check(cfg):
    setup = run.read_json(SETUP)
    assert setup["canary"]["postgres"] == "17.11" and setup["canary"]["scoped_role"] == "parity_subject"
    assert run.read_json(SETUP.with_name("setup-controller-release.json"))["exit_status"] == 0
    proof = run.read_json(KEY/"proof.json")
    assert run.digest(CANDIDATE) == proof["candidate_sha256"] == KEY_SHA
    assert proof["source"] == REVISION and proof["stock_sha256"] == run.digest(KEY/"stock.sql")
    validate(CANDIDATE.read_text())
    a, b = (proof["variants"][name] for name in ("stock", "composite"))
    assert a["contract"] == b["contract"] and a["concurrent_claims"] == b["concurrent_claims"]
    assert not run.read_json(KEY/"evidence/native/final-replay.json")["next_peer_gate"]["passed"]
    source = subprocess.check_output(["git", "show", f"{REVISION}:compiler/mesh-rt/src/jobs/schema.sql"],
                                     cwd=ROOT, text=True)
    assert source.count(STOCK) == 1
    subprocess.run(["bash", "-n", HERE/"launch.sh"], check=True)
    for file in HERE.glob("*.py"):
        compile(file.read_bytes(), str(file), "exec")
    compile(observer((LIVE/"observe.py").read_text()), "staged-observe.py", "exec")
    for name, data in public_files(cfg).items():
        if name == "native-run":
            for lane in DIAGNOSTIC + TRIPLE:
                module = setup["remote"] if lane in DIAGNOSTIC else "-"
                rendered = launcher(data.decode(), Path("/sealed/source"), lane, module)
                subprocess.run(["bash", "-n"], input=rendered, text=True, check=True)
        else:
            compile(data, name, "exec")


def public_files(cfg):
    with tarfile.open(PRODUCT/"source.tar.gz") as archive:
        prefix = f"source/{PRODUCT.relative_to(ROOT)}/replay"
        files = {name: archive.extractfile(f"{prefix}/{name}").read()
                 for name in ("native-run", "identity.py", "product_arm.py")}
    files["backend.py"] = CLOSURE_BACKEND.read_bytes()
    pins = cfg["lanes"]["plan-stock"]["inventory"]["engines"]["mesh"]["verified_files"]
    for name, data in files.items():
        expected, = [sha for path, sha in pins.items() if path.endswith(f"/replay-closure-retry/{name}")]
        assert sql_digest(data.decode()) == expected, name
    return files


def pg_cpu(proof, clocks, pids):
    submit, end = clocks["first_submit_us"], clocks["last_settlement_us"]
    a, b = proof["before"], proof["after"]
    assert a["begin_us"] <= a["end_us"] <= submit < end <= b["begin_us"] <= b["end_us"]
    assert proof["clock_ticks"] == 100
    postmaster = str(proof["postmaster"])
    first, last = a["rows"], b["rows"]
    assert all(v[0] == int(postmaster) for rows in (first, last) for pid, v in rows.items() if pid != postmaster)
    paired = {pid for pid in first.keys() & last.keys() if first[pid][1] == last[pid][1]}
    assert postmaster in paired and set(map(str, pids)) <= paired
    assert paired == first.keys() == last.keys(), "unmatched PostgreSQL processes in CPU bracket"
    deltas = [last[pid][2] - first[pid][2] for pid in paired]
    assert min(deltas) >= 0
    return dict(ms=sum(deltas)*10, stable_pids=len(paired), unmatched=[],
                slack_us=submit-a["begin_us"]+b["end_us"]-end)


def score_arm(folder, trial, lane, cfg):
    inventory = cfg["lanes"][lane]["inventory"]
    base = run.score_arm(folder, trial, "measured", "mesh", inventory)
    assert [base["host_boot_id"], base["host_kernel"], base["pg_mount"]["maj:min"]] == cfg["host"]
    path = folder/"raw.tar.gz"
    probe = lane in DIAGNOSTIC
    key = lane in {"plan-key", "key"}
    with tarfile.open(path) as archive:
        clocks = workload(archive)
        for field in ("first_submit_us", "first_commit_us", "last_settlement_us"):
            assert clocks[field] == base[field]
        overlay = member(archive, "logical-overlay")
        assert overlay["lane"] == lane and overlay["observer"] is probe
        assert overlay["baseline_body_sha256"] == sql_digest(body(STOCK))
        assert overlay["body_sha256"] == sql_digest(body(CANDIDATE.read_text() if key else STOCK))
        if probe:
            plans = diagnostic(archive, clocks, overlay, run.read_json(SETUP), key)
            return dict(plans=plans, claim_calls=clocks["claim_calls"],
                        queue_lock_timeouts=clocks["queue_lock_timeouts"], raw_sha256=run.digest(path))
        assert not any(name.startswith("adapter/grant-") for name in archive.getnames())
        postgres = pg_cpu(member(archive, "pg-cpu-bracket"), clocks,
                          [row["backend"] for row in member(archive, "postgres-tls-sessions")])
    host = host_cpu(folder, base, cfg["clock_ticks"])
    return dict(base, **{k: v for k, v in clocks.items() if k not in base}, pg_cpu=postgres,
                pg_cpu_ms=postgres["ms"], runner_cpu_ms=1024*base["runner_cpu_ms_per_job"],
                host_cpu_ms=1024*(host["pinned_user_cpu_ms_per_job"]+host["pinned_kernel_cpu_ms_per_job"]),
                raw_sha256=run.digest(path))


def score_block(folder, trial, cfg):
    assert run.read_json(folder/"order.json") == trial
    arms = {lane: score_arm(folder/f"measured-{lane}/measured-mesh", trial, lane, cfg)
            for lane in trial["order"]}
    result = dict(arms=arms)
    if trial["group"] == 0:
        result["plan_gate"] = plan_gate(*(arms[lane]["plans"] for lane in DIAGNOSTIC))
    return result


def replay(cfg):
    complete, failures, repeats = {}, {}, {}
    for trial in SCHEDULE:
        group = trial["group"]
        for path in sorted((OUT/"blocks").glob(f"{run.block_prefix(trial)}.*")):
            try:
                result = score_block(path, trial, cfg)
            except (AssertionError, RuntimeError, OSError, ValueError, KeyError, StopIteration, tarfile.TarError) as error:
                failures[path.name] = dict(error=f"{type(error).__name__}: {error}", raw_sha256={
                    str(p.relative_to(path)): run.digest(p) for p in sorted(path.rglob("raw.tar.gz"))})
            else:
                if group in complete:
                    repeats[path.name] = result
                else:
                    complete[group] = result
    gate = pilot_gate(complete)
    release = QUEUE/f"{ID}-groups-0-2.released.json"
    released = run.read_json(release) if release.exists() else None
    return dict(groups=complete, failures=failures, additional_complete_attempts=repeats, pilot_gate=gate,
                release=released, fair_8x8_peers_eligible=bool(
                    released and released["exit_status"] == 0 and gate["passed"] and not failures and not repeats))


def active():
    assert (QUEUE/f"{ID}.json").read_bytes() == PACKET.read_bytes()
    journal = (QUEUE/"STATUS.md").read_text()
    kind, name, start, end = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                                        journal, re.M)[-1]
    assert (kind, name) == ("TIMING", ID)
    assert datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
    return dict(id=ID, start=start, end=end, request_sha256=run.digest(PACKET))


def dispatch(cfg):
    grant = active()
    OUT.mkdir(parents=True, exist_ok=True)
    run.write_json(OUT/f"grant-{time.time_ns()}.json", grant)
    with (SUITE/".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
        staging = OUT/"staging"/str(time.time_ns())
        for lane in DIAGNOSTIC + TRIPLE:
            folder = staging/lane
            folder.mkdir(parents=True)
            inventory = folder/"inventory.json"
            run.write_json(inventory, cfg["lanes"][lane]["inventory"])
            with (folder/"stage.log").open("xb") as log:
                for source in (*FILES, inventory):
                    active()
                    run.stage(source, remote(lane), log)
        for trial in SCHEDULE:
            path = OUT/"blocks"/f"{run.block_prefix(trial)}.{time.time_ns()}"
            path.mkdir(parents=True)
            run.write_json(path/"order.json", trial)
            for lane in trial["order"]:
                remaining = datetime.fromisoformat(active()["end"]) - datetime.now(timezone.utc)
                assert remaining.total_seconds() >= 125, "controller time budget exhausted"
                folder = path/f"measured-{lane}"
                folder.mkdir()
                run.dispatch_arm(folder, remote(lane), trial, "measured", "mesh")
                assert run.read_json(folder/"measured-mesh/dispatch.json").get("exit_code") == 0, lane
            result = score_block(path, trial, cfg)
            run.write_json(path/"reconciled.json", result)
            if trial["group"] == 0 and not result["plan_gate"]["passed"]:
                print("Live 8×8 locked target gate held. Ending after qualification.", flush=True)
                break
    return 0


def intake(seal):
    assert seal == run.digest(PACKET)
    assert (QUEUE/f"{ID}.json").read_bytes() == PACKET.read_bytes(), "register the exact packet first"
    released = QUEUE/f"{ID}-groups-0-2.released.json"
    if released.exists():
        return run.read_json(released)["exit_status"]
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) ", (QUEUE/"STATUS.md").read_text(), re.M)
    assert events[-1] == "RELEASED", "another grant still owns the controller"
    packet = run.read_json(PACKET)
    packet.update(timing_grant_id=ID, command_attempts=1)
    return runpy.run_path(str(QUEUE.parent/"completion-writer-r2.py"))["measure"](
        packet, "timing_command", [0, 1, 2])


def main(args):
    cfg = config()
    stage_check(cfg)
    if args == ["seal"]:
        assert not PACKET.exists()
        run.write_json(PACKET, request(cfg))
        return 0
    assert run.read_json(PACKET) == request(cfg), "sealed inputs changed"
    if args == ["check"]:
        print(f"{ID} verified {run.digest(PACKET)}")
        return 0
    if args == ["replay"]:
        print(json.dumps(replay(cfg), sort_keys=True, indent=2))
        return 0
    if args == ["timing"]:
        return dispatch(cfg)
    if len(args) == 2 and args[0] == "intake":
        return intake(args[1])
    raise SystemExit("usage: controller.py seal|check|replay|timing|intake SHA")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
