"""Validate the sealed pilot offline or run it as the sole Linode controller."""
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

from launcher import rewrite
from layout import GUARD_SOURCE_SHA, LANES, guard_sql
from prove import BATCH, INPUT, REVISION, ROOT, frozen_sql
from wal import reduce

HERE = Path(__file__).resolve().parent
RESULTS = HERE.parent
SUITE = RESULTS.parent
ID = HERE.name
PACKET = HERE / "controller-packet.json"
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
OUT = HERE / "evidence/native"
TRANSFER = RESULTS / "2026-11-24-dense-product-d2-transfer"
BASELINE = TRANSFER / "evidence/native-closure-retry"
ORIGINAL = RESULTS / "2026-11-24-trusted-jobs-product-composition"
FILES = ("launch.sh", "launcher.py", "observe.py", "layout.py", "wal.py")
BLOCKS = [dict(group=i, profile="noop", phases=["warmup", "measured"], order=list(order))
          for i, order in enumerate((LANES, LANES[::-1]), 1)]
ELF = "af2aca867a02d526cb6ba92d1867166ffc346f31e97a8f8aba0f19aa76b1d355"
MANIFEST = "979cd601099c5e7da4be763229a91e9dcabca51a35f4bed79f6cf4128adbac55"
FLOOR = {"clock_fraction": 0.05, "cpu_regression_fraction": 0.10,
         "footprint_regression_fraction": 0.25, "enqueue_extent_regression_fraction": 1.10}

sys.path[:0] = [str(SUITE), str(TRANSFER),
                str(RESULTS / "2026-11-23-bulk-enqueue-v2-short/replay")]
matrix = import_module("interleave-v2")
run = matrix.run
dense = import_module("transfer")
need = run.require


def configuration():
    config = run.read_json(BASELINE / "series.json")["config"]
    original = run.read_json(BASELINE / "staging/hot/inventory.json")
    config["description"] = "Frozen public grouped hot Mesh 2x32, jobs fillfactor 50 and duplicate defaults"
    config["contract"] = "ordinary-trusted-shared-process-v2"
    config["lanes"] = {}
    for lane in LANES:
        inventory = deepcopy(original)
        spec = inventory["engines"]["mesh"]
        argv = spec["argv"]
        old = Path(argv[-2])
        stage = run.REMOTE / f"{ID}-{lane}"
        need(argv[-1] == "mesh" and old.name == "native-run"
             and "PRODUCT_COMPOSITION=product-d2" in argv and "PRODUCT_TOPOLOGY=two-runner" in argv
             and f"PRODUCT_ELF_SHA256={ELF}" in argv and spec["artifact_sha256"] == MANIFEST
             and spec["version"] == REVISION, "archived hot inventory drift")
        pins = {p: v for p, v in spec["verified_files"].items()
                if not (p.startswith(str(run.REMOTE) + "/") and p.endswith("/worker.py"))}
        pins[str(stage / "worker.py")] = run.digest(SUITE / "worker.py")
        pins[str(old.parents[5] / "compiler/mesh-rt/src/jobs/enqueue_schema.sql")] = GUARD_SOURCE_SHA
        pins.update({str(stage / name): run.digest(HERE / name) for name in FILES})
        spec.update(argv=[*argv[:-2], "/bin/bash", str(stage / "launch.sh"), str(old), lane, "mesh"],
                    verified_files=pins)
        config["lanes"][lane] = dict(engine="mesh", runners=2, inventory=inventory)
    matrix.validate(config)
    return config


def structure():
    data = json.loads(gzip.decompress((HERE / "evidence/structure.json.gz").read_bytes()))
    need(data["postgres"] == "17.11" and data["input_sha256"] == INPUT and data["revision"] == REVISION
         and data["guard_sha256"] == sha256(guard_sql(frozen_sql("enqueue_schema.sql")).encode()).hexdigest(),
         "structural proof provenance changed")
    summary = {}
    for lane in LANES:
        arm = data["arms"][lane]
        need(len(arm["rejections"]) == 10 and all(value == ["f", "0|stock", "0|0|0"]
                                                 for value in arm["rejections"].values()),
             "layout guard negative proof changed")
        cohorts = []
        for cohort in arm["cohorts"]:
            outcome = {key: cohort[key] for key in ("wrong_token_settled", "available_heap_bytes", "heap_bytes", "total_relation_bytes")}
            need(outcome["wrong_token_settled"] == 0, "wrong token changed the lease")
            enqueue = reduce("\n".join(cohort["enqueue_waldump"]))
            need(enqueue["inserts"] == 16, "structural bulk enqueue did not log 16 inserts")
            outcome["enqueue_extent_pages"] = enqueue["insert_extent_pages"]
            for index, phase in enumerate(("claim", "complete"), 1):
                totals = reduce("\n".join(cohort[phase + "_waldump"]))
                totals["full_argument_prefix"] = sum(
                    item["prefix"] >= cohort["snapshots"][index][key]["args_end"]
                    for key, item in cohort[phase].items())
                need(totals["updates"] == 16 and totals["hot"] == 0, "unexpected structural record type")
                if lane == "fill50":
                    need(totals["same_page"] == totals["full_argument_prefix"] == 16,
                         "fillfactor did not produce the complete on-page prefix")
                elif phase == "claim":
                    need(totals["same_page"] < 16, "default no longer shows cross-page claims")
                outcome[phase] = totals
            cohorts.append(outcome)
        need(len(cohorts) == 2, "cold or turnover cohort missing")
        summary[lane] = cohorts
    return summary


def packet():
    value = run.read_json(PACKET)
    need(value["id"] == ID and value["input_sha256"] == INPUT and value["schedule"] == BLOCKS
         and value["gate"] == FLOOR and value["only_existing_controller"] is True,
         "pilot contract changed")
    for name, checksum in value["local_sha256"].items():
        need(run.digest(ROOT / name) == checksum, f"changed pilot input {name}")
    return value


def score(folder, phase, lane, config):
    path = folder / "raw.tar.gz"
    result = dense.score_arm(folder, phase, "hot", config, None)
    with tarfile.open(path) as raw:
        def get(name):
            return run.file_json(raw, f"adapter/{name}.json")

        before, after, identity = map(get, ("layout-before", "layout-end", "layout-identity"))
        need(identity["lane"] == lane and identity["waldump_version"] == "pg_waldump (PostgreSQL) 17.11"
             and identity["guard_sha256"] == sha256(guard_sql(frozen_sql("enqueue_schema.sql")).encode()).hexdigest(),
             "layout or WAL tool receipt changed")
        need(before["captured_us"] < result["first_submit_us"] < result["last_settlement_us"] < after["captured_us"]
             and after["terminal_token_state_ok"], "capture overlapped timing or job token survived")
        static = ("relation", "system_id", "reloptions", "wal_level", "wal_compression", "full_page_writes",
                  "fsync", "synchronous_commit", "track_wal_io_timing")
        need(all(before[k] == after[k] for k in static) and before["lsn"] != after["lsn"]
             and before["observer_tls"] and after["observer_tls"]
             and before["other_clients"] == after["other_clients"] == 0
             and after["reloptions"] == (["fillfactor=50"] if lane == "fill50" else None),
             "changed cluster, session, layout, or WAL envelope")
        with raw.extractfile("adapter/jobs-wal.log") as stream:
            wal = reduce(stream.read().decode())
        need(wal == get("layout-wal") and (wal["inserts"], wal["updates"]) == (1024, 2048), "incomplete queue heap WAL")
        need(get("layout-shutdown")["clean"] and before["database_stats_reset"] == after["database_stats_reset"],
             "unclean shutdown or reset between the off-clock snapshots")
        policy = get("queue-policy")
        need(not [row for row in policy["queue_statements"]
                  if "FROM mesh_jobs.claim" in row["query"] and "claim_batch_set_v1(" not in row["query"]],
             "unexpected fallback claim")
        runner_cpu_ms = 1000 * get("ledger")["runner_cpu_seconds"]
    return dict(result, physical=after, start=before, layout_identity=identity, whole_heap_wal=wal,
                runner_cpu_ms=runner_cpu_ms,
                host_cpu_ms_per_job=result["pinned_user_cpu_ms_per_job"] + result["pinned_kernel_cpu_ms_per_job"])


def contrast(rows):
    a, t, b = (rows[label] for label in LANES)
    for key in ("wal_level", "wal_compression", "full_page_writes", "fsync",
                "synchronous_commit", "track_wal_io_timing"):
        need(a["physical"].get(key) == t["physical"].get(key) == b["physical"].get(key),
             f"server setting drift: {key}")
    for key in ("guard_sha256", "guard_acl", "waldump_sha256", "waldump_version"):
        need(a["layout_identity"][key] == t["layout_identity"][key] == b["layout_identity"][key],
             f"arm identity drift: {key}")

    def gain(values, fraction):
        first, candidate, last = values
        base = min(first, last)
        spread = abs(first - last)
        return dict(candidate=candidate, faster_control=base, control_spread=spread,
                    improvement=base - candidate, passes=base - candidate > max(fraction * base, spread))

    gains = {}
    for suffix, origin in (("submit", "first_submit_us"), ("drain", "first_commit_us")):
        gains[suffix] = gain([r["last_settlement_us"] - r[origin] for r in (a, t, b)], FLOOR["clock_fraction"])
        gains["last_effect_" + suffix] = gain([r["last_effect_us"] - r[origin] for r in (a, t, b)], 0)

    def bounded(values, fraction):
        first, candidate, last = values
        limit = max(first, last) * (1 + fraction)
        return dict(candidate=candidate, limit=limit, passes=candidate <= limit)

    bounds = {k: bounded([r[k] for r in (a, t, b)], FLOOR["cpu_regression_fraction"])
              for k in ("runner_cpu_ms", "host_cpu_ms_per_job")}
    bounds.update({k: bounded([r["physical"][k] for r in (a, t, b)], FLOOR["footprint_regression_fraction"])
                   for k in ("heap_bytes", "total_relation_bytes")})
    for key, limit in (("heap_extent_pages", "footprint_regression_fraction"),
                       ("insert_extent_pages", "enqueue_extent_regression_fraction")):
        bounds[key] = bounded([r["whole_heap_wal"][key] for r in (a, t, b)], FLOOR[limit])
    mechanism = all(t["whole_heap_wal"][key] > max(a["whole_heap_wal"][key], b["whole_heap_wal"][key])
                    for key in ("same_page", "prefix"))
    return dict(advance=mechanism and all(r["passes"] for r in (*gains.values(), *bounds.values())),
                gains=gains, bounds=bounds, same_page_prefix_gain=mechanism)


def replay(path, config, block):
    need(run.read_json(path / "order.json") == block, "rotation order changed")
    arms = {}
    # Reuse the public hot scorer with this arm's inventory, including its executable and CPU checks.
    original_stages = dense.stages
    try:
        for phase, lane, folder in matrix.arms(path, config, block):
            inventory = config["lanes"][lane]["inventory"]
            dense.stages = lambda *_: ([], inventory)
            arms[f"{phase}-{lane}"] = score(folder, phase, lane, config)
    finally:
        dense.stages = original_stages
    return dict(arms=arms, decision=contrast({lane: arms[f"measured-{lane}"] for lane in LANES}))


def check_gate():
    sample = dict(first_submit_us=100, first_commit_us=200, last_settlement_us=1200,
                  last_effect_us=1100, runner_cpu_ms=100, host_cpu_ms_per_job=1,
                  layout_identity=dict(guard_sha256="guard", guard_acl=[],
                                       waldump_sha256="wal", waldump_version="17.11"),
                  physical=dict(heap_bytes=100, total_relation_bytes=100),
                  whole_heap_wal=dict(same_page=100, prefix=100, heap_extent_pages=100, insert_extent_pages=50))
    rows = {lane: deepcopy(sample) for lane in LANES}
    control = rows["default-b"]
    control["last_effect_us"] += 20
    control["last_settlement_us"] += 20
    candidate = rows["fill50"]
    candidate.update(last_settlement_us=1100, last_effect_us=1000,
                     whole_heap_wal=dict(same_page=150, prefix=150, heap_extent_pages=100, insert_extent_pages=100))
    need(contrast(rows)["advance"], "positive decision control failed")
    scratches = (
        ("last_settlement_us", 1195),
        ("last_effect_us", 1090),
        ("runner_cpu_ms", 111),
        ("host_cpu_ms_per_job", 1.11),
        ("physical", dict(heap_bytes=126, total_relation_bytes=100)),
        ("whole_heap_wal", dict(same_page=150, prefix=100, heap_extent_pages=100, insert_extent_pages=100)),
        ("whole_heap_wal", dict(same_page=150, prefix=150, heap_extent_pages=126, insert_extent_pages=100)),
        ("whole_heap_wal", dict(same_page=150, prefix=150, heap_extent_pages=100, insert_extent_pages=106)),
    )
    for key, value in scratches:
        rejected = deepcopy(rows)
        rejected["fill50"][key] = value
        need(not contrast(rejected)["advance"], f"pilot advanced despite failing {key}")
    return len(scratches)


def check(config):
    subprocess.run(["bash", "-n", HERE / "launch.sh"], check=True)
    for path in HERE.glob("*.py"):
        compile(path.read_text(), str(path), "exec")
    with tarfile.open(ORIGINAL / "source.tar.gz") as bundle:
        path = next(p for p in bundle.getnames() if p.endswith("/replay/native-run")
                    and "trusted-jobs-product-composition/" in p)
        old = bundle.extractfile(path).read().decode()
        for lane in LANES:
            argv = config["lanes"][lane]["inventory"]["engines"]["mesh"]["argv"]
            shell = rewrite(old, Path(argv[-3]).parents[5], lane)
            subprocess.run(["bash", "-n"], input=shell, text=True, check=True)
    need(sha256(sha256(gzip.decompress(BATCH.read_bytes())).hexdigest().encode()).hexdigest() == INPUT,
         "canonical dense payload changed")
    baseline = run.read_json(BASELINE / "series.json")["config"]
    historical = sorted(BASELINE.glob("blocks/*/measured-hot/measured-mesh/raw.tar.gz"))[0].parent
    original = dense.stages
    try:
        dense.stages = lambda *_: ([], run.read_json(BASELINE / "staging/hot/inventory.json"))
        prior = dense.score_arm(historical, "measured", "hot", baseline, None)
    finally:
        dense.stages = original
    need(prior["completed"] == 1024 and prior["checkout_probes"] == 4,
         "historical raw hot binding did not reconcile")
    print(json.dumps(dict(structural=structure(), archived_hot_raw_sha256=prior["raw_sha256"],
                          rejected_gate_controls=check_gate()), indent=2))


def grant():
    queued = QUEUE / f"{ID}.json"
    need(queued.is_file() and queued.read_bytes() == PACKET.read_bytes(),
         "controller has not registered this packet")
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                        (QUEUE / "STATUS.md").read_text(), re.M)
    mode, name, start, end = events[-1]
    need((mode, name) == ("TIMING", ID)
         and datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end),
         "requires the active matching controller TIMING grant")
    return dict(id=ID, start=start, end=end, request_sha256=run.digest(PACKET))


def timing(config):
    active = grant()
    OUT.mkdir(parents=True, exist_ok=True)
    now = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%f")
    run.write_json(OUT / f"grant-{now}.json", active)
    series = OUT / "series.json"
    sealed = dict(config=config, plan=BLOCKS, remote={lane: f"{ID}-{lane}" for lane in LANES},
                  drivers={p.name: run.digest(p) for p in
                           (Path(matrix.__file__), SUITE / "run.py", SUITE / "verify.py", SUITE / "worker.py")})
    if not series.exists():
        run.write_json(series, sealed)
    need(run.read_json(series) == sealed, "pilot series changed")
    stage, block, arm = run.stage, matrix.dispatch_block, run.dispatch_arm

    def stage_owned(path, remote, log):
        grant()
        if path.name == "inventory.json":
            for name in FILES:
                stage(HERE / name, remote, log)
        stage(path, remote, log)

    def checked_block(*args):
        grant()
        need(block(*args), "incomplete block retained; reconcile before retry")
        return True

    def checked_arm(path, remote, group, phase, engine):
        grant()
        arm(path, remote, group, phase, engine)
        folder = path / f"{phase}-{engine}"
        raw = folder / "raw.tar.gz"
        need(raw.is_file() and run.read_json(folder / "dispatch.json").get("exit_code") == 0,
             f"first incomplete dispatch retained in {folder}")
        with tarfile.open(raw) as archive:
            need(run.file_json(archive, "status.json")["exit_code"] == 0
                 and "adapter/layout-wal.json" in archive.getnames(), f"first failed arm retained in {folder}")

    run.stage, run.dispatch_arm, matrix.dispatch_block = stage_owned, checked_arm, checked_block
    return matrix.dispatch(OUT, config, [1, 2])


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in ("check", "timing", "replay"):
        raise SystemExit("usage: controller.py check|timing|replay")
    seal = packet()
    config = configuration()
    need(config["host"] == seal["native_host"] and config["cpus"] == [0, 1], "host binding changed")
    matrix.plan = lambda *_: iter(BLOCKS)
    matrix.replay = replay
    matrix.ATTEMPT_ERRORS += (AssertionError, AttributeError)
    action = sys.argv[1]
    if action == "check":
        check(config)
    elif action == "timing":
        return timing(config)
    else:
        failures = {}
        groups = {b["group"]: matrix.select_attempt(OUT, config, b, failures) for b in BLOCKS}
        release_path = OUT / "controller-release.json"
        release = run.read_json(release_path) if release_path.is_file() else {}
        released = release.get("id") == ID and release.get("exit_status") == 0
        advance = released and all(g and g[1]["decision"]["advance"] for g in groups.values())
        print(json.dumps(dict(groups=groups, failures=failures, controller_released=released,
                              opens_four_engine_pricing=advance), indent=2))
        return 0 if all(groups.values()) else 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
