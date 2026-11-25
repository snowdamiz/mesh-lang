"""Offline qualification and sole-controller dispatch for the recovery pilot."""
from datetime import datetime, timezone
import gzip
from importlib import import_module
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile

from contract import (BASE, BOOT, CANDIDATE, DEPTHS, ELF, HERE, INPUT, MANIFEST,
                      PHASES, POSTGRES, REVISION, ROOT, STOCK, definitions, digest, rewrite)
from probe import target_scans

RESULTS = HERE.parent
SUITE = RESULTS.parent
ID = HERE.name
PACKET = HERE / "controller-packet.json"
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
OUT = HERE / "evidence/native"
CANDIDATE_FILE = HERE.with_name("2026-11-25-empty-recovery") / "candidate.sql"
FROZEN = RESULTS / "2026-11-24-dense-product-d2-transfer"
HOT = FROZEN / "evidence/native-closure-retry"
PUBLIC = RESULTS / "2026-11-24-trusted-jobs-product-composition"
BASE_PACKET = HOT / "staging/hot/inventory.json"
BATCH = RESULTS / "2026-11-24-public-dense-set-claim/evidence/batch_000000.json.gz"
STAGED = [HERE / name for name in ("launch.sh", "observe.py", "contract.py", "probe.py")] + [CANDIDATE_FILE]
ORDERS = (PHASES, ("stock", "candidate"), ("candidate", "stock"))
BLOCKS = [dict(group=i, profile="noop", phases=["measured"], order=list(order))
          for i, order in enumerate(ORDERS, 1)]
FLOOR = dict(retained_ms_per_call=0.10, retained_blocks_per_call=50,
             dense_sql_ms=10, dense_submit_fraction=0.05,
             empty_ms_per_call=0.02, due_ms_per_call=0.075)

sys.path[:0] = [str(SUITE), str(FROZEN), str(RESULTS / "2026-11-23-bulk-enqueue-v2-short/replay")]
matrix = import_module("interleave-v2")
dense = import_module("transfer")
run = matrix.run
need = run.require


def config():
    cfg = run.read_json(HOT / "series.json")["config"]
    cfg.update(description="One predicate in public dense hot grouped Mesh recover(), fresh PG17.11 2x32",
               contract="ordinary-trusted-shared-process-v2", lanes={})
    for lane in PHASES:
        inventory = run.read_json(BASE_PACKET)
        spec = inventory["engines"]["mesh"]
        argv = spec["argv"]
        old = argv[-2]
        stage = run.REMOTE / f"{ID}-{lane}"
        need("PRODUCT_COMPOSITION=product-d2" in argv and "PRODUCT_TOPOLOGY=two-runner" in argv
             and "PRODUCT_STORAGE=default" in argv and f"PRODUCT_ELF_SHA256={ELF}" in argv
             and spec["artifact_sha256"] == MANIFEST and spec["version"] == REVISION, "archived hot lane changed")
        pins = spec["verified_files"]
        # The worker is re-staged at this experiment's unique path.
        for key in list(pins):
            if key.startswith(str(run.REMOTE) + "/") and key.endswith("/worker.py"):
                del pins[key]
        pins[str(stage / "worker.py")] = run.digest(SUITE / "worker.py")
        pins[f"{BASE}/toolchain/pgsql-17.11/bin/postgres"] = POSTGRES
        pins.update({str(stage / p.name): run.digest(p) for p in STAGED})
        spec["argv"] = [*argv[:-2], "/bin/bash", str(stage / "launch.sh"), old, lane, "mesh"]
        cfg["lanes"][lane] = dict(engine="mesh", runners=2, inventory=inventory)
    need(cfg["host"] == [BOOT, "6.8.0-149-generic", "8:0"] and cfg["cpus"] == [0,1],
         "archived native envelope changed")
    matrix.validate(cfg)
    return cfg


def check_packet():
    packet = run.read_json(PACKET)
    need(packet["id"] == ID and packet["schedule"] == BLOCKS and packet["gate"] == FLOOR
         and packet["ready"] and packet["only_existing_controller"] and packet["no_build"]
         and packet["input_sha256"] == INPUT and packet["elf_sha256"] == ELF
         and packet["candidate_sha256"] == CANDIDATE and packet["stock_sha256"] == STOCK,
         "packet contract changed")
    for name, expected in packet["local_sha256"].items():
        need(run.digest(ROOT / name) == expected, f"changed packet input {name}")
    return packet


def proof(cfg):
    candidate = definitions(CANDIDATE_FILE.read_bytes())
    stock = subprocess.check_output(["git", "-C", ROOT, "show", f"{REVISION}:compiler/mesh-rt/src/jobs/schema.sql"])
    assert stock.count(candidate["stock"].encode()) == 1
    subprocess.run(["bash","-n", HERE / "launch.sh"], check=True)
    for path in HERE.glob("*.py"):
        compile(path.read_bytes(), str(path), "exec")
    with tarfile.open(PUBLIC / "source.tar.gz") as archive:
        name, = [p for p in archive.getnames() if p.endswith("/replay/native-run")]
        original = archive.extractfile(name).read().decode()
        for label in PHASES:
            argv = cfg["lanes"][label]["inventory"]["engines"]["mesh"]["argv"]
            script = rewrite(original, Path(argv[-3]).parents[5], label)
            subprocess.run(["bash","-n"], input=script, text=True, check=True)
    assert digest(digest(gzip.decompress(BATCH.read_bytes())).encode()) == INPUT
    archived = sorted(HOT.glob("blocks/*/measured-hot/measured-mesh/raw.tar.gz"))[0].parent
    baseline = run.read_json(HOT / "series.json")["config"]
    historical = stock_score(archived, "measured", baseline, run.read_json(BASE_PACKET))
    local = run.read_json(HERE / "local-correctness.json")
    assert historical["completed"] == 1024 and local["stock"] == local["candidate"] and not local["timed"]
    print(json.dumps(dict(native_pilot_ready=True, host=cfg["host"], archived_hot_raw_sha256=historical["raw_sha256"],
                          verified_depths=DEPTHS, packet_sha256=run.digest(PACKET)), indent=2))


def stock_score(path, phase, cfg, inventory):
    original = dense.stages
    try:
        dense.stages = lambda *_: ([], inventory)
        return dense.score_arm(path, phase, "hot", cfg, None)
    finally:
        dense.stages = original


def per_call(record):
    summary, = record["stats"]["statements"]
    return dict(calls=summary["calls"], rows=summary["rows"],
                elapsed_ms=summary["total_exec_time"] / summary["calls"],
                blocks=(summary["shared_blks_hit"] + summary["shared_blks_read"])/summary["calls"],
                wal_bytes=float(summary["wal_bytes"])/summary["calls"])


def verify_plan_gate(value):
    result = {}
    for depth in DEPTHS:
        fixtures = [value[arm]["fixtures"][str(depth)] for arm in ("stock", "candidate")]
        old, new = fixtures
        assert all(old["history"][k] == new["history"][k] for k in
                   ("rows", "distinct_ids", "json_text_min", "json_text_max", "committed", "content_digest"))
        assert old["history"]["rows"] == depth and old["history"]["committed"]
        result[str(depth)] = cases = {}
        for case, count in (("empty",0), ("full",16), ("mixed",5), ("held",0)):
            a, b = [arm["cases"][case] for arm in fixtures]
            assert a["before"] == b["before"] and a["after"] == b["after"]
            assert set(b["plans"]) == {"force_generic_plan", "force_custom_plan"}
            for plan in b["plans"].values():
                scans = list(target_scans(plan["Plan"]))
                assert scans and bool(sum(s["Actual Loops"] for s in scans)) == bool(count)
            x,y = per_call(a), per_call(b)
            assert x["calls"] == y["calls"] == (128 if case == "empty" else 12)
            assert x["rows"] == y["rows"] == count * x["calls"]
            cases[case] = dict(stock=x, candidate=y, saved_ms_per_call=x["elapsed_ms"]-y["elapsed_ms"],
                               saved_blocks_per_call=x["blocks"]-y["blocks"])
    assert value["stock"]["concurrency"] == value["candidate"]["concurrency"]
    assert value["stock"]["concurrency"]["locked_empty"] and value["stock"]["concurrency"]["guard_enabled"]
    return result


def diagnostics(archive):
    result = run.read_json(archive / "dispatch.json")
    assert result.get("exit_code") == 0, result
    with tarfile.open(archive / "raw.tar.gz") as raw:
        return verify_plan_gate(run.file_json(raw, "adapter/recovery-plan-gate.json"))


def delta(left, right):
    keys = ("calls", "rows", "total_exec_time", "shared_blks_hit", "shared_blks_read",
            "shared_blks_dirtied", "shared_blks_written", "wal_records", "wal_fpi", "wal_bytes")
    assert left["dealloc"] == right["dealloc"] == 0 and left["reset"] == right["reset"]
    assert len(left["statements"]) <= 1 and len(right["statements"]) <= 1
    a = left["statements"][0] if left["statements"] else {}
    b = right["statements"][0] if right["statements"] else {}
    if a:
        assert a["queryid"] == b["queryid"]
    result = {key: float(b.get(key,0)) - float(a.get(key,0)) for key in keys}
    assert all(value >= 0 for value in result.values()), result
    return result


def score(path, phase, lane, cfg):
    spec = cfg["lanes"][lane]["inventory"]
    result = stock_score(path, phase, cfg, spec)
    with tarfile.open(path / "raw.tar.gz") as raw:
        get = lambda name: run.file_json(raw, f"adapter/recovery-{name}.json")
        fixture, frames, tail = get("fixture"), get("frames"), get("closed")
    assert fixture["phase"] == lane and fixture["fixture"]["layout"]
    assert fixture["installed_sha256"] == (CANDIDATE if lane == "candidate" else STOCK)
    begin, end, settled = frames
    assert all(f["begin_us"] <= f["epoch_us"] <= f["end_us"] for f in frames)
    submit, commit, terminal = (result[k] for k in ("first_submit_us","first_commit_us","last_settlement_us"))
    assert begin["end_us"] < end["begin_us"] <= end["end_us"] <= submit < commit < terminal <= settled["begin_us"]
    assert tail["committed_rows"] == sum(r["rows"] for r in tail["writer"]) == 1024
    assert sum(r["calls"] for r in tail["writer"]) == 8 and sum(r["wal_records"] for r in tail["writer"]) > 0
    idle, active, late = delta(begin,end), delta(end,settled), delta(settled,tail)
    assert active["calls"] > 0 and active["rows"] == 0
    pids = {p for p in end["cpu"].keys() & settled["cpu"].keys()
            if end["cpu"][p][0] == settled["cpu"][p][0]}
    cpu = sum(settled["cpu"][p][1] - end["cpu"][p][1] for p in pids) / 1e6
    return dict(result, fixture=fixture, recovery_idle=idle, recovery_submit_to_observer=active,
                recovery_post_observer=late, submit_ms=(terminal-submit)/1000,
                enqueue_ms=(commit-submit)/1000, drain_ms=(terminal-commit)/1000,
                stable_worker_postgres_cpu_ms=cpu if pids else None, stable_worker_postgres_pids=sorted(pids),
                recovery_elapsed_scope="nested UPDATE elapsed sums, which may overlap; not CPU")


def compare(arms, probe=None):
    stock,candidate = (arms[k] for k in ("stock","candidate"))
    s,c = (a["recovery_submit_to_observer"] for a in (stock,candidate))
    assert stock["fixture"]["fixture"] == candidate["fixture"]["fixture"]
    assert stock["fixture"]["procedure"][1:6] == candidate["fixture"]["procedure"][1:6]
    assert stock["fixture"]["procedure"][7:] == candidate["fixture"]["procedure"][7:]
    dense_saved = s["total_exec_time"] - c["total_exec_time"]
    submit_saved = stock["submit_ms"] - candidate["submit_ms"]
    result = dict(dense_sql_saved_ms=dense_saved, submit_saved_ms=submit_saved,
                  dense_material=(dense_saved >= FLOOR["dense_sql_ms"]
                      or submit_saved >= FLOOR["dense_submit_fraction"] * stock["submit_ms"]))
    if probe is not None:
        result["retained_material"] = any(
            probe[str(d)][name]["saved_ms_per_call"] >= FLOOR["retained_ms_per_call"] and
            probe[str(d)][name]["saved_blocks_per_call"] >= FLOOR["retained_blocks_per_call"]
            for d in DEPTHS[1:] for name in ("empty","held"))
        result["regression_signal"] = (
            any(-rows[name]["saved_ms_per_call"] > max(
                    FLOOR["empty_ms_per_call"], 0.15 * rows[name]["stock"]["elapsed_ms"])
                for rows in probe.values() for name in ("empty", "held"))
            or any(-rows[name]["saved_ms_per_call"] > max(
                    FLOOR["due_ms_per_call"], 0.15 * rows[name]["stock"]["elapsed_ms"])
                   for rows in probe.values() for name in ("full","mixed")))
        result["advance"] = (result["dense_material"] or result["retained_material"]) and not result["regression_signal"]
    return result


def replay(path, cfg, block):
    assert run.read_json(path / "order.json") == block
    arms, probe = {}, None
    for phase, label, folder in matrix.arms(path, cfg, block):
        if label == "gate-stock":
            # This shakedown may warm the host. Its clock stays out of the pair.
            probe = diagnostics(folder)
        else:
            arms[label] = score(folder, phase, label, cfg)
    return dict(arms=arms, probe=probe, decision=compare(arms, probe))


def grant():
    queued = QUEUE / f"{ID}.json"
    need(queued.is_file() and queued.read_bytes() == PACKET.read_bytes(), "controller packet is unregistered")
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                        (QUEUE / "STATUS.md").read_text(), re.M)
    mode, name, start, end = events[-1]
    need((mode, name) == ("TIMING", ID)
         and datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end),
         "requires the active matching Linode TIMING assignment")
    return dict(start=start, end=end, request_sha256=run.digest(PACKET))


def timing(cfg, advance):
    active = grant()
    groups = [2,3] if advance else [1]
    if advance:
        first = matrix.select_attempt(OUT,cfg,BLOCKS[0],{})
        need(first and first[1]["decision"]["advance"], "pilot signal did not qualify")
    OUT.mkdir(parents=True, exist_ok=True)
    run.write_json(OUT / f"grant-{datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S%f')}.json", active)
    series = OUT / "series.json"
    sealed = dict(config=cfg, plan=BLOCKS, remote={lane:f"{ID}-{lane}" for lane in PHASES},
                  drivers={p.name:run.digest(p) for p in
                           (Path(matrix.__file__), SUITE/"run.py", SUITE/"verify.py", SUITE/"worker.py")})
    if not series.exists():
        run.write_json(series,sealed)
    need(run.read_json(series) == sealed, "sealed series changed")
    stage, dispatch, arm = run.stage, matrix.dispatch_block, run.dispatch_arm

    def staged(path, remote, log):
        grant()
        if path.name == "inventory.json":
            for member in STAGED:
                stage(member,remote,log)
        stage(path,remote,log)

    def assigned(*args):
        grant()
        return dispatch(*args)

    def gated(path, remote, block, phase, engine):
        grant()
        arm(path, remote, block, phase, engine)
        folder = path / f"{phase}-{engine}"
        need((folder / "raw.tar.gz").is_file() and run.read_json(folder / "dispatch.json").get("exit_code") == 0,
             f"first incomplete dispatch retained in {folder}")
        if path.name.endswith("-gate-stock"):
            diagnostics(folder)

    run.stage, matrix.dispatch_block, run.dispatch_arm = staged, assigned, gated
    return matrix.dispatch(OUT, cfg, groups)


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in ("check","timing","advance","replay"):
        raise SystemExit("usage: controller.py check|timing|advance|replay")
    check_packet()
    cfg = config()
    matrix.plan = lambda *_: iter(BLOCKS)
    matrix.replay = replay
    matrix.ATTEMPT_ERRORS += (AssertionError,)
    action = sys.argv[1]
    if action == "check":
        proof(cfg)
        return 0
    if action in ("timing","advance"):
        return timing(cfg, action == "advance")
    issues = {}
    pairs = {b["group"]: matrix.select_attempt(OUT,cfg,b,issues) for b in BLOCKS}
    print(json.dumps(dict(pairs=pairs, issues=issues), indent=2, sort_keys=True))
    return 0 if pairs[1] else 1


if __name__ == "__main__":
    sys.exit(main())
