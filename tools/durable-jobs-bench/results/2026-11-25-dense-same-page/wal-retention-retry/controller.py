"""Retain the first pilot and stage its build-free WAL-retention sibling."""
from importlib.util import module_from_spec, spec_from_file_location
import json
from pathlib import Path
import sys
import tarfile

from wal_window import WALDUMP_SHA, window

HERE = Path(__file__).resolve().parent
ORIGINAL = HERE.parent
sys.path.append(str(ORIGINAL))
spec = spec_from_file_location("original_same_page_controller", ORIGINAL / "controller.py")
original = module_from_spec(spec)
spec.loader.exec_module(original)
ROOT, run, matrix, dense, need = original.ROOT, original.run, original.matrix, original.dense, original.need
ID = "2026-11-25-dense-same-page-wal-retention-retry"
OUT = HERE / "evidence/native"
PACKET = HERE / "controller-packet.json"
ORIGINAL_OUT = ORIGINAL / "evidence/native"
FILES = {"launch.sh": HERE, "observe.py": HERE, "wal_window.py": HERE,
         "launcher.py": ORIGINAL, "layout.py": ORIGINAL, "wal.py": ORIGINAL}
WALDUMP = "/var/lib/durable-jobs-bench/native/toolchain/pgsql-17.11/bin/pg_waldump"


def configuration():
    cfg = original.configuration()
    for lane, definition in cfg["lanes"].items():
        spec = definition["inventory"]["engines"]["mesh"]
        before = str(run.REMOTE / f"{original.ID}-{lane}") + "/"
        after = str(run.REMOTE / f"{ID}-{lane}") + "/"
        spec["argv"] = [after + arg.removeprefix(before) if arg.startswith(before) else arg for arg in spec["argv"]]
        remote_pins = {p: sha for p, sha in spec["verified_files"].items() if not p.startswith(before)}
        remote_pins.update({after + name: run.digest(path / name) for name, path in FILES.items()})
        remote_pins.update({after + "worker.py": run.digest(original.SUITE / "worker.py"), WALDUMP: WALDUMP_SHA})
        spec["verified_files"] = remote_pins
    matrix.validate(cfg)
    return cfg


def member(raw, name):
    return run.file_json(raw, f"adapter/{name}.json")


def failed_arm(raw):
    status = run.file_json(raw, "status.json")
    if status["exit_code"] == 0:
        return None
    if "adapter/failure.json" not in raw.getnames():
        return f"native exit {status['exit_code']} without adapter failure receipt"
    failure = member(raw, "failure")
    stage = " -> ".join(f"{name}:{line} {fn}" for name, line, fn in failure["stage"])
    detail = f"native exit {status['exit_code']}: {failure['type']} at {stage}"
    if "adapter/layout-retention-failure.json" in raw.getnames():
        detail += f"; {member(raw, 'layout-retention-failure')['detail']}"
    if "adapter/layout-decoder.log" in raw.getnames():
        stderr = raw.extractfile("adapter/layout-decoder.log").read().decode().strip()
        if stderr:
            detail += f"; {stderr[:500]}"
    return detail


def frozen_score(folder, phase, lane, config):
    inventory = config["lanes"][lane]["inventory"]
    grading = {**config, "lanes": {"hot": config["lanes"][lane]}}
    stages = dense.stages
    try:
        dense.stages = lambda *_: ([], inventory)
        return dense.score_arm(folder, phase, "hot", grading, None)
    finally:
        dense.stages = stages


def score(folder, phase, lane, config):
    path = folder / "raw.tar.gz"
    need(path.is_file(), f"missing native export for {phase}/{lane}")
    with tarfile.open(path) as raw:
        failure = failed_arm(raw)
        need(failure is None, f"{phase}/{lane}: {failure}")
        result = frozen_score(folder, phase, lane, config)
        before, after, identity = (member(raw, key) for key in ("layout-before", "layout-end", "layout-identity"))
        need(identity["lane"] == lane and identity["waldump_version"] == "pg_waldump (PostgreSQL) 17.11"
             and identity["waldump_sha256"] == WALDUMP_SHA
             and identity["guard_sha256"] == run.read_json(HERE / "evidence/guard.json")["guard_sha256"],
             "layout or WAL tool receipt changed")
        need(before["captured_us"] < result["first_submit_us"] < result["last_settlement_us"] < after["captured_us"]
             and after["terminal_lease_state_ok"], "capture overlapped timing or job lease survived")
        static = ("relation", "system_id", "reloptions", "wal_level", "wal_compression", "full_page_writes",
                  "fsync", "synchronous_commit", "track_wal_io_timing")
        need(all(before[k] == after[k] for k in static)
             and before["observer_tls"] and after["observer_tls"]
             and before["other_clients"] == after["other_clients"] == 0
             and after["reloptions"] == (["fillfactor=50"] if lane == "fill50" else None),
             "changed cluster, session, layout, or WAL envelope")
        retained = member(raw, "layout-wal-snapshot")
        need(retained["start_lsn"] == before["lsn"] and retained["end_lsn"] == after["lsn"]
             and retained["files"] == window(before["lsn"], after["lsn"], retained), "incomplete WAL snapshot")
        wal = original.reduce(raw.extractfile("adapter/jobs-wal.log").read().decode())
        need(wal == member(raw, "layout-wal") and (wal["inserts"], wal["updates"]) == (1024, 2048),
             "incomplete queue heap WAL")
        need(member(raw, "layout-shutdown")["clean"] and before["database_stats_reset"] == after["database_stats_reset"],
             "unclean shutdown or reset between the off-clock snapshots")
        policy = member(raw, "queue-policy")
        need(not [row for row in policy["queue_statements"]
                  if "FROM mesh_jobs.claim" in row["query"] and "claim_batch_set_v1(" not in row["query"]],
             "unexpected fallback claim")
        runner_cpu_ms = 1000 * member(raw, "ledger")["runner_cpu_seconds"]
    return dict(result, physical=after, start=before, layout_identity=identity, whole_heap_wal=wal,
                runner_cpu_ms=runner_cpu_ms,
                host_cpu_ms_per_job=result["pinned_user_cpu_ms_per_job"] + result["pinned_kernel_cpu_ms_per_job"])


def block_replay(path, config, block):
    need(run.read_json(path / "order.json") == block, "rotation order changed")
    arms = {f"{phase}-{lane}": score(folder, phase, lane, config)
            for phase, lane, folder in matrix.arms(path, config, block)}
    return dict(arms=arms, decision=original.contrast({lane: arms[f"measured-{lane}"] for lane in original.LANES}))


def original_failure():
    files = list(ORIGINAL_OUT.glob("blocks/*/*/*/raw.tar.gz"))
    requests = list(ORIGINAL_OUT.glob("blocks/*/*/*/request.json"))
    need(len(files) == len(requests) == 1, "original attempt inventory changed")
    path, = files
    folder = path.parent
    release = run.read_json(ORIGINAL_OUT / "controller-release.json")
    need(release["id"] == original.ID and release["exit_status"] == 1, "original release changed")
    with tarfile.open(path) as raw:
        before, after = (member(raw, key) for key in ("layout-before", "layout-end"))
        invocation = run.file_json(raw, "invocation.json")
        need(invocation["phase"] == "warmup" and invocation["profile"] == "noop", "wrong original arm")
        postgres = raw.extractfile("adapter/postgres.log").read().decode()
        need("1 recycled; write=" in postgres and f"lsn={after['lsn']}, redo lsn={after['lsn']}" in postgres,
             "shutdown recycling witness changed")
        need("adapter/jobs-wal.log" not in raw.getnames() and "adapter/ledger.json" not in raw.getnames(),
             "original archive unexpectedly contains a final WAL reduction or ledger")
        error = failed_arm(raw)
        statements = member(raw, "trusted-application-statements")
        writes = {kind: [r["rows"] for r in statements if r["query"].startswith(
            f"INSERT INTO public.mesh_jobs_parity_{kind}")] for kind in ("attempts", "effects")}
        writer = member(raw, "producer-writer")
    try:
        score(folder, "warmup", "default-a", original.configuration())
    except ValueError as exc:
        need(str(exc) == f"warmup/default-a: {error}" and "FixtureError at" in str(exc)
             and "observe.py:90 stop -> jobs-parity.py:235 call" in str(exc),
             "frozen grader masked original failure")
    else:
        raise ValueError("frozen grader accepted failed original warmup")
    return dict(raw_sha256=run.digest(path), original_released_exit=release["exit_status"], captured_arms=1,
                qualified_arms=0, measured_arms=0, qualified_groups=0, eligible_for_peer_pricing=False,
                graded_rejection=str(error), wal_bounds=[before["lsn"], after["lsn"]],
                observed_writes=writes, bulk=writer["bulk"], scalar=writer["scalar"],
                reported_heap_changes={key: after["live_relation_counters"][key]
                                       for key in ("n_tup_ins", "n_tup_upd", "n_tup_newpage_upd", "n_tup_hot_upd")})


def packet():
    parent = original.packet()
    retry = run.read_json(PACKET)
    for key in ("native_host", "cpus", "revision", "elf_sha256", "manifest_sha256", "input_sha256",
                "jobs_fillfactor", "schedule", "gate", "resource_bounds", "original_request_sha256"):
        expected = run.digest(original.PACKET) if key == "original_request_sha256" else parent[key]
        need(retry[key] == expected, f"retry changed frozen {key}")
    need(retry["id"] == ID and retry["only_existing_controller"] is True, "changed controller boundary")
    for key, seal in retry["local_sha256"].items():
        need(run.digest(ROOT / key) == seal, f"changed retry input {key}")
    return retry


def timing(config):
    original.ID, original.OUT, original.PACKET, original.FILES = ID, OUT, PACKET, tuple(FILES)
    stage = run.stage

    def stage_retry(path, remote, log):
        if path.parent == ORIGINAL and path.name in FILES:
            path = FILES[path.name] / path.name
        stage(path, remote, log)

    run.stage = stage_retry
    return original.timing(config)


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in ("check", "timing", "replay"):
        raise SystemExit("usage: controller.py check|timing|replay")
    seal = packet()
    cfg = configuration()
    need(cfg["host"] == seal["native_host"] and cfg["cpus"] == [0, 1], "host binding changed")
    matrix.plan = lambda *_: iter(original.BLOCKS)
    matrix.replay = block_replay
    matrix.ATTEMPT_ERRORS += (AssertionError, AttributeError)
    if sys.argv[1] == "timing":
        return timing(cfg)
    if sys.argv[1] == "check":
        from verify_retry import check
        return check(cfg)
    original_result = original_failure()
    failures = {}
    groups = {block["group"]: matrix.select_attempt(OUT, cfg, block, failures) for block in original.BLOCKS}
    release_path = OUT / "controller-release.json"
    release = run.read_json(release_path) if release_path.is_file() else {}
    released = release.get("id") == ID and release.get("exit_status") == 0
    advance = released and all(group and group[1]["decision"]["advance"] for group in groups.values())
    print(json.dumps(dict(original=original_result, groups=groups, failures=failures,
                          controller_released=released, opens_four_engine_pricing=advance), indent=2))
    return 0 if all(groups.values()) else 1


if __name__ == "__main__":
    sys.exit(main())
