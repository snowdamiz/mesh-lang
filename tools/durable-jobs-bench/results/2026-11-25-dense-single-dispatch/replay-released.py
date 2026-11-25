"""Recover independently valid measurements from the released, rejected pilot."""
from collections import Counter
from datetime import datetime
import inspect
import json
import tarfile
from types import FunctionType

import driver
from qualification import mesh_helpers
import worker

ROOT = driver.OUTPUT
require = driver.require
read = driver.run.read_json
CONFIG = ["search_path=pg_catalog, pg_temp", "lock_timeout=100ms"]
SCHEMA = "source/compiler/mesh-rt/src/jobs/schema.sql"
ROSTER_ERROR = "TLS readiness or measured peak changed"
REDACTED = "[redacted]"
PRIMARY = ("single-four", "stock-a", "stock-b")


def source_witness():
    # driver.check() has already bound this archive to the released ELF proof.
    with tarfile.open(driver.base.SOURCE / "source.tar.gz") as raw:
        get = driver.dense.archive_files(raw)
        schema = get(SCHEMA)
        migrate = get("source/compiler/mesh-rt/src/jobs/store.rs")
    declaration = schema.split(b"CREATE OR REPLACE FUNCTION mesh_jobs.claim_batch_set_v1(", 1)[1]
    header, body, _ = declaration.split(b"$$", 2)
    require(b"db.simple(include_str!(\"schema.sql\"))?" in migrate
            and b"SET search_path = pg_catalog, pg_temp SET lock_timeout = '100ms'" in header
            and driver.dense.digest(body) == driver.CLAIM_BODY, "source proof differs from pilot")
    match = worker.PRIVATE_VALUE.search(body.decode())
    require(match is not None and worker.redact_json({"claim_body": body.decode()}) == {"claim_body": REDACTED}
            and worker.redact_json({"claim_body_sha256": driver.CLAIM_BODY})
                == {"claim_body_sha256": driver.CLAIM_BODY},
            "could not reproduce source redaction and digest survival")
    return dict(source=driver.base.REV, body_bytes=len(body), body_sha256=driver.CLAIM_BODY,
                redactor_match=match.group(), expected_config=CONFIG,
                archived_live_body="redacted by export", live_body_digest_verified=False)


def replay_scorer():
    """Execute the remaining sealed scorer in a private scope after checking the redacted field."""
    source = inspect.getsource(driver.dispatcher_score)
    for old, new in (
        ('dense.digest(observation["claim_body"].encode()) == CLAIM_BODY',
         'observation["claim_body"] == "[redacted]"'),
        ('dense.matrix.mesh_helpers()', 'mesh_helpers()'),
        ('lock_function_body_sha256=CLAIM_BODY', 'expected_lock_function_body_sha256=CLAIM_BODY'),
    ):
        require(source.count(old) == 1, f"sealed scorer anchor differs: {old}")
        source = source.replace(old, new)
    scope = dict(vars(driver), mesh_helpers=mesh_helpers)
    exec(compile(source, "<released archive checks>", "exec"), scope)
    return FunctionType(driver.score_arm.__code__, scope)


def arm_inventory(raw, folder, phase, inventory, cfg):
    get = lambda member: driver.run.file_json(raw, member)
    status, invocation, host = (get(f"{name}.json") for name in ("status", "invocation", "host"))
    request, dispatch = (read(folder / f"{name}.json") for name in ("request", "dispatch"))
    require(all(request[key] == invocation[key] == value for key, value in
                dict(phase=phase, attempt=request["attempt"], profile="noop", engine="mesh").items())
            and status["exit_code"] == dispatch["exit_code"]
            and [host["boot_id"], host["kernel"], host["pg_mount"]["maj:min"]] == cfg["host"]
            and host["affinity"] == cfg["cpus"]
            and host["verified_files"] == inventory["engines"]["mesh"]["verified_files"],
            "archive does not match its dispatch")
    return status["exit_code"]


def inspect_arm(folder, phase, label, cfg, remote, score_archive):
    staged = ROOT / "staging" / label / "inventory.json"
    _, inventory = driver.stages(cfg, label, remote)
    require(read(staged) == inventory, "staged inventory differs from frozen plan")
    archive = folder / "raw.tar.gz"
    with tarfile.open(archive) as raw:
        get = lambda name: driver.run.file_json(raw, f"adapter/{name}.json")
        names = set(raw.getnames())
        exit_code = arm_inventory(raw, folder, phase, inventory, cfg)
        sessions = get("postgres-tls-sessions")
        counts = dict(Counter(row["role"] for row in sessions))
        setup = get("dense-setup")
        privileges = get("privileges")
        expected_roles = dict(worker_set=False, producer_set=False, application_set=False,
                              worker_cursor=True, worker_may_insert_job=False)
        require(all(row["tls"] for row in sessions) and len({row["backend"] for row in sessions}) == len(sessions)
                and counts == dict(compiled_caller=32, parity_producer=1, parity_subject=counts["parity_subject"])
                and setup == dict(storage=dict(args_storage="x", work_mem="4MB", max_connections=96),
                                  limits=dict(compiled_caller=32, parity_subject=8, parity_producer=1))
                and privileges == dict(before=expected_roles, after=dict(expected_roles, worker_set=True)),
                "TLS envelope, storage, role limits or observed grants changed")
        result = dict(raw_sha256=driver.run.digest(archive), exit_code=exit_code, ready_tls_by_role=counts,
                      role_limits=setup["limits"], privileges=privileges)
        if exit_code != 0:
            failure = get("failure")
            report = json.loads(driver.dense.text(raw, "adapter/runner-0.log").splitlines()[-1])
            require(label == "single-two" and exit_code == 1 and counts["parity_subject"] < 4
                    and failure["stage"][-1][::2] == ["adapter.py", "capture_runners"]
                    and failure["message"] == "expected 32 warmed application TLS sessions and the fixed queue and producer roster"
                    and report["claimed"] == report["finished"] == 0 and report["drained"]
                    and not names.intersection({"adapter/ledger.json", "adapter/row.json", "adapter/dispatcher-observation.json"}),
                    "unexpected native failure shape")
            return dict(result, outcome="readiness_exit", failure=failure, claimed=0)
        observation = get("dispatcher-observation")
        require(observation["claim_body"] == REDACTED and observation["claim_config"] == CONFIG,
                "archive claim body redaction or exported lock config differs")
        result.update(claim_config=observation["claim_config"], claim_body_export=observation["claim_body"])

        try:
            score = score_archive(folder, phase, label, cfg, remote)
        except ValueError as error:
            require(phase == "warmup" and label.startswith("stock-") and str(error) == ROSTER_ERROR
                    and counts["parity_subject"] < 8 and get("row")["peak_connections"] == 41,
                    f"unexpected archived measurement rejection: {error}")
            return dict(result, outcome="readiness_count_rejection", reason=str(error), sampled_peak_connections=41)
        queue_pids = {r["backend"] for r in sessions if r["role"] == "parity_subject"}
        first, last = ({r["pid"]: r["cpu_ticks"] for r in observation[boundary]["rows"]}
                       for boundary in ("before", "after"))
        require(queue_pids <= set(score["pg_cpu"]["paired_pids"]), "queue CPU participant missing from bracket")
        scale = 1000 / observation["clock_ticks"]
        queue_cpu = scale * sum(last[pid] - first[pid] for pid in queue_pids)
        return dict(result, outcome="remaining_checks_pass", queue_backend_bracket_cpu_ms=queue_cpu, **score)


def compare_primary(arms):
    one = arms["single-four"]
    a, b = arms["stock-a"], arms["stock-b"]
    keys = ("submit_us", "drain_us", "middle_effect_us", "last_effect_from_commit_us",
            "runner_cpu_ms", "queue_backend_bracket_cpu_ms", "claim_calls", "claimed_to_terminal_peak")
    clocks = {key: driver.clock(a[key], one[key], b[key]) for key in ("submit_us", "drain_us")}
    combined = lambda row: row["runner_cpu_ms"] + row["pg_cpu"]["cpu_ms"]
    comparisons = {
        label: {key: one[key] - stock[key] for key in keys} | dict(
            postgres_bracket_cpu_ms=one["pg_cpu"]["cpu_ms"] - stock["pg_cpu"]["cpu_ms"],
            runner_plus_postgres_bracket_cpu_ms=combined(one) - combined(stock))
        for label, stock in (("stock-a", a), ("stock-b", b))
    }
    earlier = one["last_effect_from_commit_us"] < min(a["last_effect_from_commit_us"], b["last_effect_from_commit_us"])
    runner_saved = min(a["runner_cpu_ms"], b["runner_cpu_ms"]) - one["runner_cpu_ms"]
    combined_saved = min(combined(a), combined(b)) - combined(one)
    return dict(candidate_minus_control=comparisons, clocks=clocks, earlier_last_effect=earlier,
                runner_cpu_saved_ms=runner_saved, combined_bracket_cpu_saved_ms=combined_saved,
                runner_savings_share_of_historical_target=[runner_saved / 512, runner_saved / 337],
                primary_observed_thresholds_met=all(check["passes"] for check in clocks.values())
                and earlier and runner_saved > 0 and combined_saved > 0)


def replay():
    cfg, pins = driver.check()
    grant_path, = ROOT.glob("grant-*.json")
    grant = read(grant_path)
    release = read(ROOT / "controller-release.json")
    release_copy, = ROOT.glob("controller-timing-*.json")
    original = read(ROOT / "summary.json")
    summary_copy, = ROOT.glob("summary-*.json")
    series = read(ROOT / "series.json")
    require(grant["id"] == release["id"] == driver.ID and grant["mode"] == "TIMING"
            and grant["request_sha256"] == pins["request"]
            and release == read(release_copy) and original == read(summary_copy)
            and release["exit_status"] == 1 and release["groups"] == [1, 2]
            and datetime.fromisoformat(grant["start"]) <= datetime.fromisoformat(release["started"])
                < datetime.fromisoformat(release["released_at"]) <= datetime.fromisoformat(grant["end"])
            and original["missing"] == [1, 2] and original["advance_to_peers"] is False
            and all(series[k] == value for k, value in dict(config=cfg, pins=pins, schedule=driver.SCHEDULE).items()),
            "released grant, series or original rejection differs")
    witness = source_witness()
    score_archive = replay_scorer()
    blocks = {}
    for block in driver.SCHEDULE:
        attempt, = ROOT.glob(f"blocks/{driver.run.block_prefix(block)}.*")
        require(read(attempt / "order.json") == block, "native arm order changed")
        evidence = {"primary": {}, "secondary": {}, "warmups": {}}
        for phase, label, _, folder in driver.matrix.paths(attempt, cfg, block):
            row = inspect_arm(folder, phase, label, cfg, driver.run.REMOTE / series["remotes"][label], score_archive)
            section = "warmups" if phase == "warmup" else ("primary" if label in PRIMARY else "secondary")
            evidence[section][label] = row
        try:
            driver.replay_block(attempt, block, cfg, series["remotes"])
        except ValueError as error:
            require(str(error) == "live SQL claim lock or function differs from public source"
                    and original["failures"][attempt.name] == f"ValueError: {error}",
                    "original rejection differs")
        else:
            raise ValueError("unexpected original qualification")
        primary = evidence["primary"]
        require(set(primary) == set(PRIMARY) and all(row["outcome"] == "remaining_checks_pass" for row in primary.values())
                and all(a["last_settlement_us"] < b["first_submit_us"]
                        for a, b in zip(list(primary.values()), list(primary.values())[1:])),
                "primary measurements incomplete or overlapping")
        blocks[str(block["group"])] = dict(attempt=attempt.name, **evidence, contrast=compare_primary(primary))

    return dict(witness=witness, release=release, original_verdict=original, blocks=blocks,
                advance_to_peers=False, decision="retain 2x32 fixed-two")


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True, allow_nan=False))
