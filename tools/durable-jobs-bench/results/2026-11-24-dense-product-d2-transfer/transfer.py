#!/usr/bin/env python3
"""Bind and replay the dense product checkout transfer. Timing needs the Linode controller."""
import argparse
from collections import Counter
from copy import deepcopy
from datetime import datetime, timezone
import hashlib
from importlib import import_module
from itertools import permutations, pairwise
import json
from pathlib import Path
import re
from statistics import median
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
RESULTS = HERE.parent
ROOT = HERE.parents[3]
ID = HERE.name
PRODUCT = "2026-11-24-trusted-jobs-product-composition"
PRODUCT_REVISION = "b3557273d72be3802bf189b284e7caaf5f3caa0f"
PRODUCT_SOURCE = "623c46b92cc5f94eb19d455e36e6fca1e1b800231441a6de480e2c35aa091b02"
REQUEST = HERE / "request.json"
PLAN = HERE / "plan.json"
PEER_ROOT = RESULTS / "2026-11-23-dense-matched-topology"
R2_ROOT = RESULTS / "2026-11-24-dense-two-runner-checkout"
OUTPUT = HERE / "evidence/native"
BASE = Path("/var/lib/durable-jobs-bench/native")
FROZEN = BASE / "source-bulk-enqueue-3b09dc3d0e23/ordinary-v2"
POSTGRES = str(BASE / "toolchain/pgsql-17.11/bin/postgres")
PG_SHA = "b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1"
COMPILER_SHA = "e023d5944764878969ca40b7edb59c779498a435b44d308776076f4c7f2e8c19"
PEERS = ("river-2x32", "oban-2x32", "apalis-2x32")
CONTROLS = ("checked-a", "checked-b")
ROLES = dict(compiled_caller=32, parity_subject=8, parity_producer=1)
TARGET = Path("tools/durable-jobs-bench/results") / PRODUCT

sys.path.insert(0, str(PEER_ROOT))
import matrix
from matrix import CPUS, INPUT, JOBS, QUEUE, SUITE, diagnostic_reconciler, require, run, text
from interleave import host_cpu
from qualification import check_affinity
from verify import xid

sys.path.append(str(R2_ROOT))
child_identity = import_module("native-score").child_identity

base_stages, base_score = matrix.stages, matrix.score_arm
matrix.ERRORS += (AssertionError,)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def request():
    subprocess.run(["openssl", "pkeyutl", "-verify", "-pubin", "-inkey", HERE / "request-public.pem",
                    "-rawin", "-in", REQUEST, "-sigfile", HERE / "request.sig"],
                   check=True, stdout=subprocess.DEVNULL)
    packet = run.read_json(REQUEST)
    require(packet["id"] == ID and packet["build_free"] is True and packet["source_sha"] == PRODUCT_REVISION,
            "unexpected signed request")
    for path, pin in packet["local_sha256"].items():
        require(run.digest(ROOT / path) == pin, f"changed signed input {path}")
    return packet


def check_original():
    history = run.read_json(HERE / "dense-r2-evidence.json")
    require(history["qualifying_groups"] == [1, 2, 4, 6] and len(history["raw_arms"]) == 121
            and history["first_failure_sha256"] in history["raw_arms"].values(), "dense trigger is incomplete")
    for path, seal in {history["archive"]: history["archive_sha256"], **history["raw_arms"],
                       **history["releases"]}.items():
        require(run.digest(ROOT / path) == seal, f"changed original dense evidence {path}")


def schedule(plan):
    blocks = plan["blocks"]
    labels = {*CONTROLS, "hot", *PEERS}
    orders = [block["order"] for block in blocks]
    require(len(orders) == 6 and all(len(order) == 6 and set(order) == labels for order in orders)
            and len({pair for order in orders for pair in pairwise(order)}) == 30
            and all(set(column) == labels for column in zip(*orders, strict=True)), "unbalanced rotations")
    for triplet in (PEERS, (*CONTROLS, "hot")):
        require({tuple(label for label in order if label in triplet) for order in orders}
                == set(permutations(triplet)), "missing triplet order")
    require(all(block["group"] == group and block["profile"] == "noop"
                and block["phases"] == ["warmup", "measured"] for group, block in enumerate(blocks, 1))
            and plan["input_sha256"] == INPUT, "wrong dense workload")
    return blocks


def archive_files(archive, prefix=""):
    members = {}
    for member in archive.getmembers():
        path = member.name.removeprefix("./")
        if not member.isfile() or not path.startswith(prefix):
            continue
        require(path not in members, f"duplicate archive member {path}")
        members[path] = member
    return lambda name: archive.extractfile(members[prefix + name]).read()


def setup_binding(release_path, raw_path):
    """Read archived product SETUP, with no ELF, workstation build, or host access."""
    release = run.read_json(release_path)
    raw_sha = run.digest(raw_path)
    require(release["id"] == PRODUCT and release["exit_status"] == 0
            and release.get("raw_sha256", raw_sha) == raw_sha, "need the successful product SETUP release")
    with tarfile.open(raw_path, "r:gz") as raw:
        get = archive_files(raw)
        manifest_bytes = get("mesh.json")
        build, witness = json.loads(manifest_bytes), json.loads(get("witness.json"))
        for item in (build, witness):
            require((item["system"], item["architecture"], item["version"], item["source_sha256"],
                     item["compiler_sha256"], item["execution_contract"]) ==
                    ("Linux", "x86_64", PRODUCT_REVISION, PRODUCT_SOURCE, COMPILER_SHA,
                     "ordinary-trusted-shared-process-v2"), "SETUP build does not match the signed product")
            require(all(re.fullmatch(r"[0-9a-f]{64}", seal) for seal in item["files"].values())
                    and re.fullmatch(r"[0-9a-f]{64}", item["runtime_sha256"]), "invalid artifact digest")
        require(build["runtime_sha256"] == witness["runtime_sha256"]
                and build["files"]["mesh"] != witness["files"]["mesh"]
                and build["files"]["writes.mpl"] == witness["files"]["writes.mpl"],
                "teardown witness must share the product runtime and handler writes")
        source = RESULTS / PRODUCT / "source.tar.gz"
        require(run.digest(source) == PRODUCT_SOURCE, "changed product archive")
        with tarfile.open(source, "r:gz") as tree:
            source_file = archive_files(tree, f"source/{TARGET}/")
            for name in ("main.mpl", "writes.mpl", "worker_identity.mpl", "worker_identity.c"):
                require(digest(source_file(f"fixture/{name}")) == build["files"][name], "fixture hash changed")
        elf_header = get("elf.txt").decode()
        require("ELF64" in elf_header and ("X86-64" in elf_header or "x86-64" in elf_header),
                "SETUP did not produce an AMD64 ELF")
        toolchain = get("toolchain.txt").decode()
        require(all(field in toolchain for field in ("rustc 1.99.0", "21.1.8", "17.11",
                                                      "90ec1c24-08c8-4bc1-aa28-79b6db1bd13a",
                                                      "6.8.0-149-generic", "8:0")), "SETUP host or tools changed")

        names = ("sql-cut", "public-handler", "ambiguous-application-write", "ambiguous-claim-commit",
                 "ambiguous-group-commit", "cancel-and-retry", "teardown-release", "teardown-kill")
        files = {name: get(f"semantic-root/semantic/{name}/result.json") for name in names}
        proof = {name: json.loads(data) for name, data in files.items()}
        require(proof["sql-cut"]["rolled_back"] and proof["sql-cut"]["wrong_token_untouched"]
                and proof["sql-cut"]["expired_untouched"] and proof["sql-cut"]["legacy_mix"],
                "SQL cut changed")
        app = proof["ambiguous-application-write"]
        require(app["wire"]["idle"] and app["wire"]["receipt_dropped"] and app["app_insert_calls"] == 3
                and proof["public-handler"]["app_insert_calls"] == 2, "same-path application fault missing")
        claim = proof["ambiguous-claim-commit"]
        require(claim["wire"]["receipt_dropped"] and claim["held"]["attempt"] == 1
                and claim["held"]["attempts"] == [] and claim["done"]["attempt"] == 2, "claim recovery missing")
        group = proof["ambiguous-group-commit"]
        require(group["wire"]["idle"] and group["wire"]["receipt_dropped"] and group["wire"]["rows"] == 2
                and group["grouped_submission_count"] == group["scalar_submission_count"] == 1,
                "same-path grouped submission proof missing")
        require(proof["cancel-and-retry"]["cancel"]["state"] == "cancelled"
                and proof["cancel-and-retry"]["retry"]["attempt"] == 2, "cancel or retry proof missing")
        for name, forced in (("teardown-release", False), ("teardown-kill", True)):
            row = proof[name]
            require(row["forced"] is forced and row["runner_sha256"] == witness["files"]["mesh"]
                    and row["held"]["state"] == "executing" and row["held"]["effect"]
                    and row["held"]["attempts"] == row["done"]["attempts"]
                    and row["held"]["effect"] == row["done"]["effect"]
                    and (row["done"] == row["held"] if forced else row["done"]["state"] == "completed"),
                    "same-runtime teardown proof missing")
    return dict(product=PRODUCT_REVISION, source_sha256=PRODUCT_SOURCE, setup_sha256=raw_sha,
                release_sha256=run.digest(release_path), manifest_sha256=digest(manifest_bytes),
                build=build, setup_proof={name: digest(data) for name, data in files.items()})


def configurations(binding):
    equal = run.read_json(PEER_ROOT / "equal.json")
    matrix.import_equal().check_config(equal)
    equal["description"] = "Trusted product dense checkout transfer, separate from the selected product series."
    equal["lanes"] = {label: equal["lanes"][label] for label in PEERS}
    equal["lanes"].update({label: dict(engine="mesh", runners=2) for label in (*CONTROLS, "hot")})
    equal["product"] = binding
    original = run.read_json(R2_ROOT / "evidence/native-import-retry/series.json")["config"]
    require(all(equal[k] == original[k] for k in ("host", "cpus", "clock_ticks", "river_equal")),
            "equal peers do not match the dense R2 campaign")
    return equal


def stages(config, label, remote):
    if label in PEERS:
        files, out = base_stages(config, label, remote)
        out["engines"][config["lanes"][label]["engine"]]["verified_files"][POSTGRES] = PG_SHA
        run.check_inventory(out)
        return files, out
    binding = config["product"]
    build = binding["build"]
    attempt = f"trusted-jobs-product-{binding['source_sha256'][:12]}"
    artifact = BASE / "builds" / attempt
    source = BASE / f"source-{attempt}" / "source"
    driver = source / TARGET / "replay"
    setup_archive = RESULTS / PRODUCT / "source.tar.gz"
    with tarfile.open(setup_archive, "r:gz") as archive:
        get = archive_files(archive, "source/")
        source_pins = {
            str(source / member): digest(get(str(member))) for member in (
                *(TARGET / "replay" / name for name in ("native-run","product_arm.py","backend.py","identity.py")),
                Path("scripts/fixtures/jobs-role-grants.sql"),
            )
        }
    frozen = RESULTS / "2026-11-24-bulk-d2-short-peers/short.json"
    template = run.read_json(frozen)["lanes"]["bulk"]["inventory"]["engines"]["mesh"]
    spec = deepcopy(template)
    spec.pop("run_as", None)
    mode = run.read_json(PLAN)["modes"][label]
    spec.update(argv=["/usr/bin/env", f"PRODUCT_ARTIFACT={artifact}", f"PRODUCT_COMPOSITION={mode}",
                      "PRODUCT_TOPOLOGY=two-runner", "PRODUCT_STORAGE=default",
                      f"PRODUCT_ELF_SHA256={build['files']['mesh']}",
                      f"PRODUCT_MANIFEST_SHA256={binding['manifest_sha256']}", str(driver / "native-run"), "mesh"],
                artifact_sha256=binding["manifest_sha256"], version=PRODUCT_REVISION,
                verified_files={path: pin for path, pin in spec["verified_files"].items()
                                if path.startswith(str(FROZEN) + "/")})
    spec["verified_files"].update(source_pins | {
        str(artifact / name): pin for name, pin in build["files"].items()
    } | {str(artifact / "mesh.json"): binding["manifest_sha256"], POSTGRES: PG_SHA,
         str(remote / "worker.py"): run.digest(SUITE / "worker.py")})
    inventory = dict(config["inventory"], engines={**config["inventory"]["engines"], "mesh": spec})
    run.check_inventory(inventory)
    return [SUITE / "worker.py"], inventory


def score_arm(folder, phase, label, config, remote):
    _, inventory = stages(config, label, remote)
    lane = config["lanes"][label]
    engine = lane["engine"]
    path = folder / "raw.tar.gz"
    if label in PEERS:
        score = base_score(folder, engine, 2, phase, inventory, config)
        with tarfile.open(path) as raw:
            ledger = run.file_json(raw, "adapter/ledger.json")
        return dict(score, last_effect_us=max(job["effect_committed_us"] for job in ledger["jobs"]))

    spec = inventory["engines"]["mesh"]
    req = run.read_json(folder / "request.json")
    with tarfile.open(path) as raw:
        def get(name):
            return run.file_json(raw, f"adapter/{name}.json")

        invocation = run.file_json(raw, "invocation.json")
        require(all(invocation[k] == req[k] == expected for k, expected in
                    dict(profile="noop", phase=phase, engine="mesh", attempt=req["attempt"]).items())
                and run.file_json(raw, "status.json")["exit_code"] == 0, "arm failed or wrong invocation")
        host = run.file_json(raw, "host.json")
        require([host["boot_id"], host["kernel"], host["pg_mount"]["maj:min"]] == config["host"]
                and host["affinity"] == CPUS and len(host["available_cpus"]) == 4
                and host["verified_files"] == spec["verified_files"], "host, affinity or files changed")
        build, ledger, row, settings, details, queue = map(get, (
            "build", "ledger", "row", "postgres-settings", "storage-topology", "queue-policy"))
        require(build == config["product"]["build"] and ledger["phase"] == row["phase"] == phase
                and ledger["local_smoke"] is row["local_smoke"] is False
                and ledger["database"] == settings and settings["version"] == "17.11"
                and row["runners"] == row["peak_processes"] == 2
                and row["poll_ms"] == 250 and row["connection_cap"] == 96
                and row["completed"] == row["reconciled_jobs"] == JOBS
                and row["committed_work_records"] == 2 * JOBS
                and row["durable_enqueue_transactions"] == 1, "scope, durability or product artifact changed")
        require(details["topology"] == "two-runner" and details["storage"] == "default"
                and details["work_mem"] == "4MB" and details["args_storage"] == "x"
                and details["max_connections"] == 96, "product storage or topology drift")
        score = diagnostic_reconciler(2)(ledger, "mesh", "noop",
                                        {key: spec[key] for key in ("version", "artifact_sha256")})
        origin, = ledger["transactions"]
        require(origin["submitted_us"] < origin["committed_us"] <= origin["acknowledged_us"],
                "submit or broker boundary changed")
        require(get("database") == dict(commit_timestamps=True, database_connection_limit=96, durable=True,
                                        max_connections=96, reserve=0, superuser_reserve=0,
                                        tls=True, user="parity_producer"), "broker/database proof changed")

        broker, *runners = get("process-identity")
        executable = next(path for path in spec["verified_files"] if "/builds/trusted-jobs-product-" in path
                          and path.endswith("/mesh"))
        launch_parent = broker["parent_pid"]
        for record, verb in [(broker, "session"), *((record, "run") for record in runners)]:
            child_identity(record, executable, build["files"]["mesh"], verb, launch_parent)
        pids = {runner["pid"] for runner in runners}
        require(len(runners) == len(pids) == 2 and broker["pid"] not in pids, "wrong runner/broker roster")
        check_affinity(get("process-affinity"), pids)
        tls = get("postgres-tls-sessions")
        roles = Counter(item["role"] for item in tls)
        require(all(item["tls"] for item in tls) and len(tls) == len({item["backend"] for item in tls})
                and roles["compiled_caller"] == ROLES["compiled_caller"] and roles["parity_producer"] == 1
                and set(roles) == set(ROLES) and 4 <= roles["parity_subject"] <= 8
                and len(tls) <= row["peak_connections"] == score["peak_connections"] <= 96,
                "warm application TLS sessions or observed physical count changed")
        jobs = ledger["jobs"]
        by_pid = Counter(job["worker_pid"] for job in jobs)
        require(by_pid.keys() == pids, "compiled handlers did not execute in both runners")
        for index, runner in enumerate(runners):
            report = json.loads(text(raw, f"adapter/runner-{index}.log").splitlines()[-1])
            slots = report["queues"]["default"]
            require(report["drained"] and report["state"] == slots["state"] == "stopped"
                    and report["last_error"] is None
                    and not any(report[k] for k in ("undrained", "recovered", "stale_acks", "pending_claims"))
                    and report["claimed"] == report["finished"] == report["outcomes"]["completed"] == by_pid[runner["pid"]]
                    and all(n == 0 for key, n in report["outcomes"].items() if key != "completed")
                    and slots["limit"] == 32 and slots["drained"] and slots["in_flight"] == slots["pending_claims"] == 0,
                    "runner exceeded its slot cap or failed to drain")

        cohorts = Counter(xid(item["settled_xid"], "terminal") for item in jobs)
        effects = {xid(job[key], key) for job in jobs for key in ("attempt_xid", "effect_xid")}
        require(len(effects) == 2 * JOBS and not effects & cohorts.keys()
                and xid(origin["xid"], "enqueue") not in effects | cohorts.keys(),
                "application, enqueue or terminal transaction overlaps")
        require(queue["queue_policy"] == ledger["queue_policy"] == row["queue_policy"] == "success_cohort"
                and queue["admitted_queries"] > 0 and len(cohorts) == queue["terminal_xids"]
                and 1 <= max(cohorts.values()) == queue["largest_terminal_cohort"] <= 8,
                "grouped-success contract or checked admission changed")
        enqueue = get("producer-writer")
        require(enqueue["enabled"] and (enqueue["bulk"], enqueue["scalar"]) == (8, 0),
                "product dense enqueue route changed")
        statements = get("trusted-application-statements")
        for suffix in ("attempts", "effects"):
            writes, = (stmt for stmt in statements
                       if stmt["query"].startswith("INSERT INTO public.mesh_jobs_parity_" + suffix))
            require(writes["calls"] == writes["rows"] == JOBS, "application commit counts changed")
        health = [stmt for stmt in statements if stmt["query"] == "SELECT $1"]
        probes = sum(stmt["calls"] for stmt in health)
        require(len(health) <= 1 and len(statements) == 2 + len(health)
                and probes == sum(stmt["rows"] for stmt in health)
                and get("raw")["raw_application_sql_calls"] == 2 * JOBS + probes
                and (0 <= probes < 2 * JOBS if label == "hot" else probes == 2 * JOBS),
                "actual checkout policy did not match the assigned lane")
        with raw.extractfile("host-samples.jsonl") as stream:
            score.update(run.sample_window(host, [json.loads(line) for line in stream], score))
    return dict(score, **host_cpu(folder, score, config["clock_ticks"]), raw_sha256=run.digest(path),
                checkout_probes=probes, probe_sql_exec_ms=sum(stmt["execution_ms"] for stmt in health),
                physical_tls_by_role=dict(roles), jobs_by_runner=[by_pid[record["pid"]] for record in runners],
                terminal_xids=len(cohorts), largest_terminal_cohort=max(cohorts.values()),
                application_sql_exec_ms=sum(stmt["execution_ms"] for stmt in statements),
                last_effect_us=max(item["effect_committed_us"] for item in jobs))


def replay_block(path, block, config, remotes):
    require(run.read_json(path / "order.json") == block, "rotation order changed")
    return {f"{phase}-{label}": score_arm(folder, phase, label, config, run.REMOTE / remotes[label])
            for phase, label, _, folder in matrix.paths(path, config, block)}


def contrast(rows, floor):
    candidate = rows["hot"]
    controls = [rows[label] for label in CONTROLS]
    clocks = {"submit": "first_submit_us", "drain": "first_commit_us"}
    deltas = {}
    for key, origin in clocks.items():
        elapsed = [row["last_settlement_us"] - row[origin] for row in [candidate, *controls]]
        faster = min(elapsed[1:])
        spread = abs(elapsed[1] - elapsed[2])
        deltas[key] = dict(hot_elapsed_us=elapsed[0], faster_control_elapsed_us=faster,
                           control_spread_us=spread,
                           gain_vs_faster_percent=100 * (faster / elapsed[0] - 1),
                           clears_gate=faster - elapsed[0] > max(spread, floor * faster))
    effect = min(row["last_effect_us"] - row["first_commit_us"] for row in controls) - (
        candidate["last_effect_us"] - candidate["first_commit_us"])
    return dict(clocks=deltas, last_effect_saving_us=effect,
                qualifies=effect > 0 and all(row["clears_gate"] for row in deltas.values()))


def summarize(directory, config, blocks, pins):
    manifest = run.read_json(directory / "series.json")
    require(all(manifest[k] == value for k, value in
                dict(config=config, schedule=blocks, pins=pins).items()), "series or binding changed")
    failures, selected, missing = {}, {}, []
    gate = run.read_json(PLAN)["gate"]
    for block in blocks:
        group = block["group"]
        found = matrix.pick(directory, block, config, manifest["remotes"], failures)
        if found is None:
            missing.append(group)
            continue
        attempt, scores = found
        rows = {label: scores[f"measured-{label}"] for label in block["order"]}
        selected[group] = dict(attempt=attempt, scores=scores,
                               contrast=contrast(rows, gate["minimum_faster_control_elapsed_fraction"]))
    qualified = [group for group, row in selected.items() if row["contrast"]["qualifies"]]
    metrics = ("submit_jobs_per_s", "commit_jobs_per_s", "submit_to_settlement_p99_ms",
               "commit_to_settlement_p99_ms", "peak_connections")
    paired = {
        metric: {peer: sum(row["scores"]["measured-hot"][metric] > row["scores"][f"measured-{peer}"][metric]
                           for row in selected.values()) for peer in PEERS}
        for metric in metrics[:2]
    }
    arms = {label: {metric: median(row["scores"][f"measured-{label}"][metric] for row in selected.values())
                    for metric in metrics} for label in config["lanes"]} if selected else {}
    paired_control = {}
    for clock in ("submit", "drain"):
        gains = [row["contrast"]["clocks"][clock]["gain_vs_faster_percent"] for row in selected.values()]
        if gains:
            paired_control[clock] = dict(min=min(gains), median=median(gains), max=max(gains))
    raw = {str(path.relative_to(directory)): run.digest(path)
           for path in sorted((directory / "blocks").rglob("raw.tar.gz"))}
    return dict(qualified_rotations=len(selected), qualifying_groups=qualified, missing=missing, failures=failures,
                advances=not missing and len(qualified) >= gate["minimum_groups"], blocks=selected, raw=raw,
                paired_hot_vs_faster_control_percent=paired_control, hot_peer_wins=paired, medians=arms)


def grant(_=None):
    mode, identifier, start, end = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                                               (QUEUE / "STATUS.md").read_text(), re.MULTILINE)[-1]
    require((mode, identifier) == ("TIMING", ID)
            and datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end),
            "dedicated controller TIMING grant required")
    require(REQUEST.read_bytes() == (QUEUE / f"{ID}.json").read_bytes(), "controller request bytes changed")
    return dict(start=start, end=end, request_sha256=run.digest(REQUEST))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("check")
    for mode in ("bind", "timing", "replay"):
        action = commands.add_parser(mode)
        action.add_argument("release", type=Path)
        action.add_argument("setup_archive", type=Path)
        if mode != "bind":
            action.add_argument("binding", type=Path)
            action.add_argument("--output", type=Path, default=OUTPUT)
        if mode == "timing":
            action.add_argument("groups", type=int, nargs="+", choices=range(1, 7))
    args = parser.parse_args()
    packet = request()
    check_original()
    blocks = schedule(run.read_json(PLAN))
    if args.command == "check":
        print("Signed request, six balanced rotations, and all 121 original raw-arm hashes verified.")
        return 0

    binding = setup_binding(args.release, args.setup_archive)
    binding["request_sha256"] = run.digest(REQUEST)
    if args.command == "bind":
        print(json.dumps(binding, indent=2, sort_keys=True))
        return 0
    require(run.read_json(args.binding) == binding, "binding does not reproduce from the archived SETUP")
    config = configurations(binding)
    matrix.stages, matrix.grant, matrix.replay_block, matrix.summarize = stages, grant, replay_block, summarize
    matrix.SERIES[ID] = ID
    pins = dict(request=run.digest(REQUEST), binding=run.digest(args.binding), **packet["local_sha256"])
    if args.command == "replay":
        result = summarize(args.output, config, blocks, pins)
        print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))
        return bool(result["missing"])
    grant()
    require(args.groups in ([1, 2], [3, 4, 5, 6]), "use the preregistered pilot or continuation")
    if args.groups[0] == 3:
        require(not {1, 2} & set(summarize(args.output, config, blocks, pins)["missing"]), "pilot must reconcile")
    underlying = run.dispatch_arm

    def dispatch_checked(path, remote, block, phase, engine):
        underlying(path, remote, block, phase, engine)
        folder = path / f"{phase}-{engine}"
        require(run.read_json(folder / "dispatch.json").get("exit_code") == 0, "retain failed native arm")
        score_arm(folder, phase, path.name.removeprefix(phase + "-"), config, remote)

    run.dispatch_arm = dispatch_checked
    args.output.parent.mkdir(exist_ok=True, parents=True)
    matrix.dispatch(args.output, config, ID, blocks, pins, args.groups)
    return bool(set(args.groups) & set(summarize(args.output, config, blocks, pins)["missing"]))


if __name__ == "__main__":
    sys.exit(main())
