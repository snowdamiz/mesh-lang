"""Verify, dispatch with a sole-controller grant, and replay the logical-key trial."""
from copy import deepcopy
from datetime import datetime, timezone
import fcntl
from itertools import islice, permutations
import json
from pathlib import Path
import re
import runpy
import shlex
import subprocess
import sys
import tarfile
import time

from contract import STOCK, body, digest as sql_digest, validate

HERE = Path(__file__).resolve().parent
RESULTS = HERE.parent
SUITE = RESULTS.parent
ROOT = SUITE.parent.parent
ID = HERE.name
PEERS_ID = ID + "-peers"
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
PACKET = HERE / "controller-packet.json"
OUT = HERE / "evidence/native"
BASE = RESULTS / "2026-11-24-dense-product-d2-transfer"
SERIES = BASE / "evidence/native-closure-retry"
LIVE = RESULTS / "2026-11-25-dense-live-grant-plans"
RECOVERY = LIVE / "procfd-recovery"
SETUP = LIVE / "evidence/native/setup.json"
STOCK_SCORE = RECOVERY / "evidence/native/blocks/01-noop/reconciled.json"
STOCK_RAW = STOCK_SCORE.parent / "sampled/measured-mesh/raw.tar.gz"
PRODUCT = RESULTS / "2026-11-24-trusted-jobs-product-composition"
SOURCE = PRODUCT / "source.tar.gz"
CANDIDATE = HERE / "candidate.sql"
CANDIDATE_SHA = "d388baa3b67c07df1a2498cd1fdc60bf7d0ce11f4b42d950f7b0a64a551c0f45"
HOST = ["90ec1c24-08c8-4bc1-aa28-79b6db1bd13a", "6.8.0-149-generic", "8:0"]
PEER_LABELS = ("river-2x32", "oban-2x32", "apalis-2x32")
DIAGNOSTIC = ("plan-stock", "plan-key")
TRIPLE = ("stock-a", "key", "stock-b")
MESH = DIAGNOSTIC + TRIPLE
FILES = tuple(HERE / f for f in ("launch.sh", "stage.py", "logical_arm.py", "contract.py", "stock.sql",
                                 "candidate.sql")) + tuple(LIVE / f for f in ("observe.py", "settings.py",
                                 "launcher.py", "plans.py")) + tuple(RECOVERY / f for f in ("adapt.py", "roster.py"))
sys.path[:0] = [str(BASE), str(LIVE), str(RECOVERY), str(SUITE)]
import transfer
from roster import verify_frames
from plans import reduce
from stage import observer, render

run = transfer.run
need = run.require


def block(group, order, phases=("measured",)):
    return dict(group=group, profile="noop", phases=list(phases), order=list(order))


PRIMARY = [block(0, DIAGNOSTIC), block(1, TRIPLE), block(2, TRIPLE[::-1])]
# Four cyclic orders and the reverse of the opening order.
peer_order = ("key", *PEER_LABELS)
PEER_TRIALS = [block(i+3, peer_order[i:] + peer_order[:i], ("warmup", "measured")) for i in range(4)]
PEER_TRIALS.append(block(7, peer_order[::-1], ("warmup", "measured")))
ALL = PRIMARY + PEER_TRIALS


def inventory_path(label):
    return SERIES / "staging" / label / "inventory.json"


def remote(label):
    return run.REMOTE / f"{ID}-{label}"


def config():
    setup = run.read_json(SETUP)
    base = run.read_json(SERIES / "series.json")["config"]
    need(base["host"] == HOST and base["cpus"] == [0, 1], "host or CPU mask changed")
    base["lanes"] = {}
    for label in (*MESH, *PEER_LABELS):
        is_mesh = label in MESH
        engine = "mesh" if is_mesh else {"river-2x32": "go", "oban-2x32": "oban", "apalis-2x32": "rust"}[label]
        inv = run.read_json(inventory_path("hot" if is_mesh else label))
        spec = inv["engines"][engine]
        pins = {p: v for p, v in spec["verified_files"].items()
                if not (p.startswith(str(run.REMOTE) + "/") and p.endswith("/worker.py"))}
        pins[str(remote(label) / "worker.py")] = run.digest(SUITE / "worker.py")
        if is_mesh:
            argv = spec["argv"]
            need("PRODUCT_COMPOSITION=product-d2" in argv and "PRODUCT_TOPOLOGY=two-runner" in argv and
                 "PRODUCT_STORAGE=default" in argv and argv[-1] == "mesh", "frozen hot launcher drift")
            module = setup["remote"] if label in DIAGNOSTIC else "-"
            spec["argv"] = [*argv[:-2], "/bin/bash", str(remote(label)/"launch.sh"), argv[-2], label, module, "mesh"]
            pins.update({str(remote(label) / f.name): run.digest(f) for f in FILES})
            if label in DIAGNOSTIC:
                pins[module] = setup["sha256"]
        spec["verified_files"] = pins
        run.check_inventory(inv)
        base["lanes"][label] = dict(engine=engine, runners=2, inventory=inv)
    return base


def point_key(scan):
    cond = scan.get("Index Cond", "")
    return (scan.get("Index Name") == "jobs_pkey" and
            re.search(r"\b\w+\.namespace\s*=\s*\w+\.grant_namespace\b", cond) and
            re.search(r"\b\w+\.id\s*=\s*\w+\.id\b", cond))


def source_check():
    proof = run.read_json(HERE / "proof.json")
    actual = run.digest(CANDIDATE)
    assert actual == proof["candidate_sha256"] == CANDIDATE_SHA
    assert proof["stock_sha256"] == run.digest(HERE / "stock.sql")
    assert proof["source"] == transfer.PRODUCT_REVISION and proof["postgres"].split()[0] == "17.11"
    schema = subprocess.check_output(["git", "show", f"{transfer.PRODUCT_REVISION}:compiler/mesh-rt/src/jobs/schema.sql"],
                                     text=True, cwd=ROOT)
    assert schema.count(STOCK) == 1
    validate(CANDIDATE.read_text())
    stock, key = (proof["variants"][name] for name in ("stock", "composite"))
    assert key["contract"] == stock["contract"] and len(key["contract"]) == 9
    assert key["concurrent_claims"] == stock["concurrent_claims"] == {
        "1": [{"i": 1, "new_context": True}],
        "2": [{"i": 1, "new_context": True}, {"i": 2, "new_context": False}]}
    plans = [targets for modes in key["plans"].values() for targets in modes.values()]
    assert len(plans) == 15 and all(len(p) == 1 and point_key(p[0]) for p in plans)
    cold = proof["cold"]
    assert len(cold["composite"]) == 1 and point_key(cold["composite"][0])
    assert "id =" not in cold["stock"][0]["Index Cond"]
    assert "id =" not in cold["namespace_only_change"][0]["Index Cond"]


def cmd(action):
    return shlex.join(["python3", "-B", str((HERE / "controller.py").relative_to(ROOT)), action])


def request():
    dispositions = [STOCK_RAW, STOCK_SCORE, SETUP, SETUP.with_name("setup-controller-release.json")]
    inputs = [*FILES, HERE/"controller.py", HERE/"proof.json", HERE/"README.md", SOURCE,
              BASE/"transfer.py", SERIES/"series.json", *dispositions,
              *(inventory_path(lane) for lane in ("hot", *PEER_LABELS)),
              *(SUITE/f for f in ("worker.py", "run.py", "verify.py", "interleave.py"))]
    return dict(id=ID, ready=True, no_build=True, after_existing_bookings=True,
                controller="native-controller-complete-post-wal-queue", cpus=[0, 1], native_host=HOST,
                candidate_sha256=CANDIDATE_SHA, input_sha256=transfer.INPUT,
                offline_check=cmd("check"), timing_command=cmd("timing"), peer_command=cmd("peers"),
                replay_command=cmd("replay"), schedule=PRIMARY, peer_schedule=PEER_TRIALS,
                timing_window_minutes=14, peer_window_minutes=26,
                evidence_destination=str(OUT), controller_receipt_dir=str(OUT/"controller"),
                local_sha256={str(f.relative_to(ROOT)): run.digest(f) for f in inputs})


def check():
    assert run.read_json(PACKET) == request(), "packet or staged dependencies changed"
    source_check()
    cfg = config()
    setup = run.read_json(SETUP)
    assert run.read_json(SETUP.with_name("setup-controller-release.json"))["exit_status"] == 0
    assert setup["canary"]["postgres"] == "17.11" and setup["canary"]["scoped_role"] == "parity_subject"
    assert re.fullmatch("[0-9a-f]{64}", setup["sha256"])
    stock = run.read_json(STOCK_SCORE)["arms"]["sampled"]
    assert run.digest(STOCK_RAW) == stock["raw_sha256"]
    baseline = plan_summary(stock["observation"], False)
    assert baseline["positive"] == 25 and baseline["broad"] == 25 and baseline["target_visits"] == 25600
    subprocess.run(["bash", "-n", HERE / "launch.sh"], check=True)
    obs = observer((LIVE / "observe.py").read_text())
    compile(obs, "staged-observe.py", "exec")
    for path in HERE.glob("*.py"):
        compile(path.read_bytes(), str(path), "exec")
    with tarfile.open(SOURCE) as archive:
        path = f"source/{PRODUCT.relative_to(ROOT)}/replay/native-run"
        raw = archive.extractfile(path).read().decode()
    for lane in MESH:
        generated = render(raw, Path("/sealed/source"), lane, setup["remote"] if lane in DIAGNOSTIC else "-")
        subprocess.run(["bash", "-n"], input=generated, text=True, check=True)
    return cfg


def plan_summary(observation, candidate):
    positive = broad = target_visits = empty = rejected = 0
    participants = set()
    for sample in observation["samples"]:
        if sample["category"] != "grant" or sample["window"] not in {"idle", "drain"}:
            continue
        ns = sample["nodes"]
        root, = [n for n in ns if not n["subtree_path"]]
        update, = [n for n in ns if n.get("Subplan Name") == "CTE granted"]
        assert update["Node Type"] == "ModifyTable" and root["emitted_approx"] == update["emitted_approx"]
        path = update["subtree_path"]
        subtree = [n for n in ns if n["subtree_path"][:len(path)] == path and n is not update]
        scans = [n for n in subtree if n.get("Relation Name") == "jobs"]
        assert scans
        visits = sum(n["emitted_approx"] + n["removed_approx"].get("Rows Removed by Filter", 0)
                     + n["removed_approx"].get("Rows Removed by Index Recheck", 0) for n in scans)
        count = root["emitted_approx"]
        if not count:
            if candidate:
                assert visits == 0 and not any(n["Actual Loops"] for n in scans)
            empty += 1
            continue
        assert sample["window"] == "drain" and 1 <= count <= 16
        expected = len(scans) == 1 and point_key(scans[0])
        if candidate:
            # Scalar point-key or an actually bounded target with bounded index input.
            bitmap = sum(n["emitted_approx"] for n in subtree if n["Node Type"] == "Bitmap Index Scan")
            assert visits <= 2*count and bitmap <= 2*count
            assert expected or (visits <= count and all(n.get("Index Cond") or n.get("Recheck Cond") for n in scans))
        tags = sample["statement_keys"]
        assert len(tags) == 1 and tags[0]["toplevel"] is False and tags[0]["role"] == "parity_subject"
        participants.add(sample["pid"])
        target_visits += visits
        rejected += sum(n["removed_approx"].get("Rows Removed by Join Filter", 0) for n in subtree)
        broad += visits > 2*count
        positive += 1
    assert positive >= 5 and empty > 0
    assert all(participants.intersection(pids) for pids in observation["participants"].values())
    return dict(positive=positive, broad=broad, target_visits=target_visits,
                empty=empty, join_rejected_approx=rejected, participant_backends=sorted(participants))


def cpu(proof, start, stop):
    a, b = proof["before"], proof["after"]
    assert a["begin_us"] <= a["end_us"] <= start < stop <= b["begin_us"] <= b["end_us"]
    assert proof["clock_ticks"] == 100
    pid = str(proof["postmaster"])
    first, last = a["rows"], b["rows"]
    assert all(v[0] == int(pid) for r in (first, last) for k, v in r.items() if k != pid)
    paired = [k for k in first.keys() & last.keys() if first[k][1] == last[k][1]]
    assert pid in paired and len(paired) >= 40, "PostgreSQL work escaped the stable CPU bracket"
    ticks = [last[k][2] - first[k][2] for k in paired]
    assert min(ticks) >= 0
    return dict(ms=sum(ticks)*10, paired=len(paired), unmatched=sorted((first.keys() | last.keys())-set(paired)),
                slack_us=start-a["begin_us"]+b["end_us"]-stop)


def score(path, phase, lane, cfg):
    inventory = cfg["lanes"][lane]["inventory"]
    # Use the retained workload reconciler with the assigned inventory.
    original = transfer.stages
    transfer.stages = lambda *_: ([], inventory)
    try:
        name = "hot" if lane in MESH else lane
        score = transfer.score_arm(path, phase, name, dict(cfg, lanes={name: cfg["lanes"][lane]}), None)
    finally:
        transfer.stages = original
    submit, enqueue, terminal = (score[key] for key in ("first_submit_us", "first_commit_us", "last_settlement_us"))
    if lane in PEER_LABELS:
        return dict(score, submit_us=terminal-submit, drain_us=terminal-enqueue,
                    raw_sha256=run.digest(path/"raw.tar.gz"))
    plan = lane in DIAGNOSTIC
    with tarfile.open(path/"raw.tar.gz") as archive:
        def get(name):
            return run.file_json(archive, f"adapter/{name}.json")
        meta = get("logical-overlay")
        stock = lane in ("plan-stock", "stock-a", "stock-b")
        assert meta["lane"] == lane and meta["observer"] is plan
        assert meta["body_sha256"] == sql_digest(body(STOCK if stock else CANDIDATE.read_text()))
        assert meta["baseline_body_sha256"] == sql_digest(body(STOCK))
        queue = get("queue-policy")
        assert queue["completion_cohort_queries"] > 0 and queue["largest_terminal_cohort"] > 1
        raw = get("raw")
        assert raw["idle_seconds"] >= 3 and raw["raw_idle_sql_calls"] > 0
        assert score["checkout_probes"] < 32 and score["peak_connections"] <= 41
        if plan:
            assert get("grant-contract")["overlay"]["body_sha256"] == meta["body_sha256"]
            assert all(get("grant-contract")["checks"].values())
            frames, closed = get("grant-frames"), get("grant-closed")
            roster = verify_frames(get("grant-roster"), frames)
            assert len(roster) == 41
            assert frames[0]["end_us"] < frames[1]["begin_us"] <= frames[1]["end_us"] < submit
            assert frames[1]["us_epoch"]-frames[0]["us_epoch"] >= 2_900_000
            assert terminal < frames[2]["us_epoch"] <= closed["us_epoch"]
            assert not any(row["usename"] in transfer.ROLES for row in closed["sessions"])
            assert frames[1]["relation"]["n_tup_ins"] == frames[1]["relation"]["n_tup_upd"] == 0
            assert closed["relation"]["n_tup_ins"] == 1024 and closed["relation"]["n_tup_upd"] == 2048
            settings = get("grant-observer")
            setup = run.read_json(SETUP)
            assert settings["module_sha256"] == setup["sha256"] and settings["module"] == setup["remote"]
            plans = get("grant-plans")
            assert all(row["pid"] in roster and roster[row["pid"]][0] == "parity_subject" for row in plans)
            proof = plan_summary(reduce(plans, frames, closed, score), not stock)
            return dict(plans=proof, raw_sha256=score["raw_sha256"], completed=score["completed"],
                        checkout_probes=score["checkout_probes"])
        assert "adapter/grant-plans.json" not in archive.getnames()
        assert "adapter/grant-observer.json" not in archive.getnames()
        db_cpu = cpu(get("pg-cpu-bracket"), submit, terminal)
    return dict(score, submit_us=terminal-submit, drain_us=terminal-enqueue, pg_cpu=db_cpu,
                runner_cpu_ms=1024*score["runner_cpu_ms_per_job"],
                host_cpu_ms=1024*(score["pinned_user_cpu_ms_per_job"]+score["pinned_kernel_cpu_ms_per_job"]))


def pilot_gate(blocks):
    result = {}
    if not {0, 1, 2} <= blocks.keys():
        return dict(passed=False, reason="qualification or pilot incomplete")
    for i in (1, 2):
        a, c, b = [blocks[i]["measured"][lane] for lane in TRIPLE]
        clocks = {}
        for metric in ("submit_us", "drain_us"):
            faster = min(a[metric], b[metric])
            spread = abs(a[metric]-b[metric])
            saving = faster-c[metric]
            clocks[metric] = dict(saving_us=saving, spread_us=spread, passed=saving>max(spread, .05*faster))
        ca, cb, cc = [item["pg_cpu"]["ms"] for item in (a, b, c)]
        ra, rb, rc = [item["runner_cpu_ms"] for item in (a, b, c)]
        resource = (cc <= max(ca, cb) and rc <= max(ra, rb) and cc+rc < min(ca+ra, cb+rb)
                    and c["host_cpu_ms"] <= max(a["host_cpu_ms"],b["host_cpu_ms"])
                    and c["peak_connections"] <= max(a["peak_connections"], b["peak_connections"]))
        result[i] = dict(clocks=clocks, no_observed_cpu_or_connection_transfer=resource,
                         passed=resource and all(v["passed"] for v in clocks.values()))
    return dict(passed=all(v["passed"] for v in result.values()), groups=result)


def score_block(folder, trial, cfg):
    assert run.read_json(folder/"order.json") == trial
    phases = {phase: {} for phase in trial["phases"]}
    for phase in trial["phases"]:
        for lane in trial["order"]:
            engine = cfg["lanes"][lane]["engine"]
            phases[phase][lane] = score(folder/f"{phase}-{lane}"/f"{phase}-{engine}", phase, lane, cfg)
    if trial["group"] == 0:
        key, stock = [phases["measured"][lane]["plans"] for lane in ("plan-key", "plan-stock")]
        prior = run.read_json(STOCK_SCORE)["arms"]["sampled"]["observation"]
        assert key["broad"] == 0 and (stock["broad"] > 0 or plan_summary(prior, False)["broad"] == 25)
    return phases


def replay(cfg):
    complete, failures, repeats = {}, {}, {}
    for trial in ALL:
        group = trial["group"]
        for path in sorted((OUT/"blocks").glob(f"{run.block_prefix(trial)}.*")):
            try:
                result = score_block(path, trial, cfg)
            except (AssertionError, RuntimeError, OSError, ValueError, KeyError, StopIteration, tarfile.TarError) as err:
                failures[path.name] = dict(error=f"{type(err).__name__}: {err}",
                    raw_sha256={str(p.relative_to(path)): run.digest(p) for p in sorted(path.rglob("raw.tar.gz"))})
            else:
                if group in complete:
                    repeats[path.name] = result
                else:
                    complete[group] = result
    return dict(groups=complete, failures=failures, additional_complete_attempts=repeats,
                next_peer_gate=pilot_gate(complete))


def active(key):
    assert (QUEUE/f"{key}.json").read_bytes() == PACKET.read_bytes()
    journal = (QUEUE/"STATUS.md").read_text()
    kind, name, start, end = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+)(?: ([^\s,]+))?(?: ([^\s,]+))?",
                                        journal, re.M)[-1]
    assert (kind, name) == ("TIMING", key)
    now = datetime.now(timezone.utc)
    assert datetime.fromisoformat(start) <= now < datetime.fromisoformat(end)
    return dict(id=key,start=start,end=end,request_sha256=run.digest(PACKET))


def clean_primary(cfg):
    released = QUEUE/f"{ID}-groups-0-2.released.json"
    assert run.read_json(released)["exit_status"] == 0 and replay(cfg)["next_peer_gate"]["passed"]


def dispatch(cfg, peer):
    key, trials = (PEERS_ID, PEER_TRIALS) if peer else (ID, PRIMARY)
    if peer:
        clean_primary(cfg)
    grant = active(key)
    OUT.mkdir(parents=True, exist_ok=True)
    run.write_json(OUT/f"grant-{time.time_ns()}.json", grant)
    lanes = sorted({lane for trial in trials for lane in trial["order"]})
    with (SUITE/".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        from interleave import HOST_IDLE
        run.ssh(["bash","-c",HOST_IDLE], timeout=12).check_returncode()
        staging = OUT/"staging"/f"{key}.{time.time_ns()}"
        staging.mkdir(parents=True)
        for lane in lanes:
            folder = staging/lane
            folder.mkdir()
            inventory = folder/"inventory.json"
            run.write_json(inventory,cfg["lanes"][lane]["inventory"])
            with (folder/"stage.log").open("xb") as log:
                for source in ((FILES if lane in MESH else ())+(SUITE/"worker.py",inventory)):
                    active(key)
                    run.stage(source, remote(lane), log)
        for trial in trials:
            path = OUT/"blocks"/f"{run.block_prefix(trial)}.{time.time_ns()}"
            path.mkdir(parents=True)
            run.write_json(path/"order.json",trial)
            for phase in trial["phases"]:
                for lane in trial["order"]:
                    end = datetime.fromisoformat(active(key)["end"])
                    assert (end-datetime.now(timezone.utc)).total_seconds() >= 125, "controller time budget exhausted"
                    engine = cfg["lanes"][lane]["engine"]
                    folder = path/f"{phase}-{lane}"
                    folder.mkdir()
                    run.dispatch_arm(folder, remote(lane), trial, phase, engine)
                    need(run.read_json(folder/f"{phase}-{engine}"/"dispatch.json").get("exit_code") == 0,
                         f"{lane} arm failed. Original attempt retained")
            run.write_json(path/"reconciled.json",score_block(path,trial,cfg))
    return 0


def intake(cfg, seal, peer):
    assert seal == run.digest(PACKET), "wrong exact request hash"
    if peer:
        clean_primary(cfg)
    key, groups = (PEERS_ID, [3,4,5,6,7]) if peer else (ID, [0,1,2])
    registered = QUEUE/f"{key}.json"
    assert registered.read_bytes() == PACKET.read_bytes(), "sole controller must register exact packet bytes"
    released = QUEUE/f"{key}-groups-{groups[0]}-{groups[-1]}.released.json"
    if released.exists():
        return run.read_json(released)["exit_status"]
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) ", (QUEUE/"STATUS.md").read_text(), re.M)
    assert events[-1] == "RELEASED", "previous holder still owns the controller"
    packet = run.read_json(PACKET)
    packet.update(timing_grant_id=key, command_attempts=1,
                  timing_window_minutes=packet["peer_window_minutes" if peer else "timing_window_minutes"])
    return runpy.run_path(str(QUEUE.parent/"completion-writer-r2.py"))["measure"](
        packet, "peer_command" if peer else "timing_command", groups)


def main(args):
    if args == ["seal"]:
        assert not PACKET.exists()
        source_check()
        run.write_json(PACKET, request())
        return 0
    cfg = check()
    if args == ["check"]:
        print(f"{ID} verified {run.digest(PACKET)}")
        return 0
    if args == ["replay"]:
        print(json.dumps(replay(cfg), indent=2, sort_keys=True))
        return 0
    if args in (["timing"], ["peers"]):
        return dispatch(cfg, args == ["peers"])
    if len(args) == 2 and args[0] in {"intake", "intake-peers"}:
        return intake(cfg, args[1], args[0] == "intake-peers")
    raise SystemExit("usage: controller.py check|replay|timing|peers|intake SHA|intake-peers SHA")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
