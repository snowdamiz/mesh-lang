"""Check or replay offline. The active Linode controller alone dispatches timing."""

from copy import deepcopy
from datetime import datetime, timezone
import gzip
from hashlib import sha256
from importlib import import_module
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile

from audit import ARCHIVE, ELF, HERE, INPUT, MANIFEST, SOURCE, digest, lifecycle, member, read
from trace import analyze

ROOT = HERE.parents[3]
SUITE = HERE.parents[1]
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
ID = HERE.name
PACKET = HERE / "controller-packet.json"
OUT = HERE / "evidence/native"
MATRIX_DIR = HERE.parent / "2026-11-23-bulk-enqueue-v2-short/replay"
sys.path[:0] = [str(SUITE), str(MATRIX_DIR)]
matrix = import_module("interleave-v2")
run = matrix.run
SCHEDULE = ("equal-off-a", "equal-trace", "equal-off-b", "stock-off-a", "stock-trace", "stock-off-b")
BLOCKS = [dict(group=i, profile="noop", phases=["measured"], order=list(order))
          for i, order in enumerate((SCHEDULE, SCHEDULE[::-1]), 1)]
FILES = ("launch.sh", "observe.py")


def plan(*_):
    return iter(BLOCKS)


def mode(label):
    return "trace" if label.endswith("-trace") else "off"


def config():
    old = {key: read(ARCHIVE / f"dense-{key}/controller-evidence/series.json")["config"]
           for key in ("equal", "stock")}
    result = deepcopy(old["equal"])
    result.update(description="Public dense Jobs set-based claim diagnostic, equal 2x32 and separate stock 8x8",
                  lanes={})
    for label in SCHEDULE:
        key = label.split("-", 1)[0]
        lane = deepcopy(old[key]["lanes"]["product"])
        spec = lane["inventory"]["engines"]["mesh"]
        assert spec["artifact_sha256"] == MANIFEST
        assert f"PRODUCT_MANIFEST_SHA256={MANIFEST}" in spec["argv"]
        assert f"PRODUCT_ELF_SHA256={ELF}" in spec["argv"]
        assert f"PRODUCT_TOPOLOGY={'two-runner' if key == 'equal' else 'stock'}" in spec["argv"]
        old_run = spec["argv"][-2]
        assert old_run.endswith("/replay-closure-retry/native-run")
        stage = Path(f"{run.REMOTE}/{ID}-{label}")
        spec["argv"][-2:-1] = ["/bin/bash", str(stage / "launch.sh"), old_run, mode(label)]
        spec["verified_files"].update({str(stage / file): digest(HERE / file) for file in FILES})
        result["lanes"][label] = lane
    return result


def inputs():
    packet = read(PACKET)
    assert packet["id"] == ID and packet["input_sha256"] == INPUT and packet["schedule"] == BLOCKS
    assert packet["no_build"] and packet["controller"] == "linode-native-stepchange-controller-v1"
    for path, sha in packet["local_sha256"].items():
        assert digest(ROOT / path) == sha, path
    cfg = config()
    assert cfg["host"] == packet["native_host"] and cfg["cpus"] == [0, 1]
    matrix.validate(cfg)
    return cfg


def check(cfg):
    subprocess.run(["bash", "-n", HERE / "launch.sh"], check=True)
    for file in HERE.glob("*.py"):
        compile(file.read_bytes(), str(file), "exec")
    data = gzip.decompress((HERE / "evidence/batch_000000.json.gz").read_bytes())
    assert sha256(sha256(data).hexdigest().encode()).hexdigest() == INPUT
    proof_path = SOURCE / "evidence/native/setup/trusted-jobs-product-623c46b92cc5-closure-retry/qualification.json"
    proof = read(proof_path)
    assert proof["executable_sha256"] == ELF and proof["manifest_sha256"] == MANIFEST
    assert digest(proof_path.with_name("raw.tar.gz")) == proof["raw_sha256"]
    assert all(proof["files"].items() <= lane["inventory"]["engines"]["mesh"]["verified_files"].items()
               for lane in cfg["lanes"].values())
    print(f"{ID}: {len(BLOCKS)} opposite orders, {len(SCHEDULE)} arms per order, qualified public ELF and batch verified")


def grant():
    assert PACKET.read_bytes() == (QUEUE / f"{ID}.json").read_bytes()
    entries = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                         (QUEUE / "STATUS.md").read_text(), re.M)
    kind, name, start, end = entries[-1]
    assert (kind, name) == ("TIMING", ID) and datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
    return dict(start=start, end=end, request=digest(PACKET))


def counter_delta(before, after):
    previous = {str(s["queryid"])+str(s["toplevel"]): s for s in before["statements"]}
    rows = []
    for row in after["statements"]:
        old = previous.get(str(row["queryid"])+str(row["toplevel"]), {})
        output = {k: row[k] for k in ("query", "queryid", "toplevel")}
        for key in ("calls","rows","total_exec_time","shared_blks_hit","shared_blks_read","temp_blks_read",
                    "temp_blks_written","wal_records","wal_bytes"):
            output[key] = row[key]-old.get(key, 0)
            assert output[key] >= 0, (key, output)
        rows.append(output)
    return rows


def cpu_delta(before, after):
    a, b = before["cpu"], after["cpu"]
    pids = [k for k in a.keys() & b.keys() if a[k][0] == b[k][0] and b[k][1] >= a[k][1]]
    return dict(cpu_ms=sum(b[k][1]-a[k][1] for k in pids)/1e6, stable_pids=sorted(pids),
                missing=sorted((set(map(str, before["pids"])) | set(map(str, after["pids"]))) - set(pids)),
                scope="stable queue-role PostgreSQL backends, all operations; observation bracket")


def score(folder, phase, label, block, cfg):
    lane = cfg["lanes"][label]
    runners = 2 if label.startswith("equal-") else 8
    profile = block["profile"]
    frozen = matrix.PROFILES[profile]
    matrix.PROFILES[profile] = (runners, *frozen[1:])
    try:
        result = run.score_arm(folder, block, phase, "mesh", lane["inventory"])
    finally:
        matrix.PROFILES[profile] = frozen
    assert [result["host_boot_id"], result["host_kernel"], result["pg_mount"]["maj:min"]] == cfg["host"]
    submit, enqueue, terminal = (result[k] for k in ("first_submit_us", "first_commit_us", "last_settlement_us"))
    with tarfile.open(folder / "raw.tar.gz") as raw:
        ledger, writer, policy = (member(raw, name) for name in ("ledger","producer-writer","queue-policy"))
        lifecycle(ledger, writer, policy)
        assert len(member(raw,"process-identity")) == runners+1
        assert member(raw, "build")["files"]["mesh"] == ELF
        assert ledger["peak_connections"] <= (41 if runners == 2 else 65)
        assert len([r for r in member(raw, "postgres-tls-sessions") if r["role"] == "compiled_caller"]) == 32
        assert all(r["tls"] for r in member(raw, "postgres-tls-sessions"))
        observe = member(raw, "claim-observer")
        assert observe["mode"] == mode(label)
        start, end, settled = member(raw, "claim-frames")
        offline = member(raw, "claim-off-clock")
        assert all(f["begin_us"] <= f["observed_us"] <= f["end_us"] for f in (start,end,settled,offline))
        assert start["end_us"] < end["begin_us"] <= end["end_us"] <= submit < enqueue < terminal <= settled["begin_us"]
        assert settled["end_us"] <= offline["begin_us"]
        assert start["calls"]["dealloc"] == end["calls"]["dealloc"] == settled["calls"]["dealloc"] == offline["dealloc"] == 0
        stats = dict(startup_to_idle=counter_delta({"statements": []}, start), idle=counter_delta(start, end),
                     submit_to_observer=counter_delta(end, settled),
                     post_observer=counter_delta(settled, offline))
        top, = [r for r in stats["submit_to_observer"] if r["toplevel"]]
        assert top["calls"] == member(raw, "raw")["raw_claim_calls"]
        if mode(label) == "trace":
            assert "pg_stat_statements.track=all" in offline["role_settings"]
            assert any(not row["toplevel"] and row["calls"] for row in stats["submit_to_observer"])
            observations = analyze(raw.extractfile("adapter/postgres.log").read().decode(),
                                   (start["observed_us"],end["observed_us"]), submit,enqueue,terminal)
            assert observations["claims"], "no claim Execute duration log records survived"
            if observe["plans_available"]:
                assert "session_preload_libraries=auto_explain" in offline["role_settings"]
                assert observations["plan_counts"].get("committed_drain/grant", 0), "no in-drain grant plans survived"
        else:
            observations = None
            assert all(r["toplevel"] for frames in stats.values() for r in frames)
    return dict(result, raw_sha256=digest(folder/"raw.tar.gz"), observer=observe, elapsed_sql=stats,
                idle_cpu=cpu_delta(start,end), submit_to_observer_cpu=cpu_delta(end,settled),
                functions_off_clock=offline["functions"], relations_off_clock=offline["relations"],
                diagnostic_log=observations,
                drain_ms=(terminal-enqueue)/1000, submit_ms=(terminal-submit)/1000)


def replay(path, cfg, block):
    assert read(path/"order.json") == block
    arms = {label: score(folder, phase, label, block, cfg)
            for phase,label,folder in matrix.arms(path,cfg,block)}
    perturbation = {}
    for key in ("equal","stock"):
        a,b,t = [arms[f"{key}-{suffix}"] for suffix in ("off-a","off-b","trace")]
        perturbation[key] = {k: dict(off_a=a[k], off_b=b[k], trace=t[k],
                                     trace_minus_off_midpoint=t[k]-(a[k]+b[k])/2)
                             for k in ("drain_ms","submit_ms")}
    return dict(arms=arms, perturbation=perturbation)


def timing(cfg):
    active = grant()
    OUT.mkdir(exist_ok=True)
    series = OUT / "series.json"
    # Pre-allocate stable stage paths for inventory SHA validation and replay.
    frozen = dict(config=cfg, plan=BLOCKS,
                  drivers={p.name: run.digest(p) for p in
                           (Path(matrix.__file__), SUITE/"run.py", SUITE/"verify.py", SUITE/"worker.py")},
                  remote={label: f"{ID}-{label}" for label in SCHEDULE})
    if not series.exists():
        run.write_json(series,frozen)
    assert read(series) == frozen
    run.write_json(OUT / f"grant-{datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S%f')}.json", active)
    stage = run.stage
    dispatch_block = matrix.dispatch_block

    def stage_files(path, remote, log):
        if path.name == "inventory.json":
            grant()
            for name in FILES:
                stage(HERE/name,remote,log)
        stage(path,remote,log)

    def assigned(*args):
        grant()
        return dispatch_block(*args)

    run.stage = stage_files
    matrix.dispatch_block = assigned
    return matrix.dispatch(OUT, cfg, [1,2])


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in ("check","timing","replay"):
        raise SystemExit("usage: controller.py check|timing|replay")
    cfg = inputs()
    matrix.plan = plan
    matrix.replay = replay
    matrix.ATTEMPT_ERRORS += (AssertionError,)
    action = sys.argv[1]
    if action == "check":
        check(cfg)
    elif action == "timing":
        return timing(cfg)
    else:
        failures = {}
        groups = {b["group"]: matrix.select_attempt(OUT,cfg,b,failures) for b in BLOCKS}
        print(json.dumps(dict(groups=groups, failed=failures),sort_keys=True,indent=2,allow_nan=False))
        return 0 if all(groups.values()) else 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
