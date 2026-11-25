"""Replay the native ledgers and build the comparison without contacting a host."""

import argparse
import hashlib
import json
import runpy
from decimal import Decimal, ROUND_HALF_UP
from pathlib import Path
from statistics import median
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
SUITE = HERE.parents[1]
sys.path.insert(0, str(SUITE))
import run


RESPONSE_METRICS = ("submit_to_settlement_p50_ms", "submit_to_settlement_p99_ms", "commit_to_settlement_p99_ms")

METRICS = (*RESPONSE_METRICS, "submit_jobs_per_s", "commit_jobs_per_s", "first_submit_to_commit_ms",
           "commit_to_pickup_p99_ms", "runner_cpu_ms_per_job", "peak_runner_rss_bytes",
           "cpu_steal_percent", "cpu_iowait_percent")


def capture_topology(directory, kind):
    sys.path.insert(0, str(HERE.parent / "2026-11-23-dense-matched-topology"))
    import matrix

    config, schedule, pins = matrix.inputs(kind)
    summary = matrix.summarize(directory, config, schedule, pins)
    if summary["missing"]:
        raise ValueError(f"{directory}: missing native rotations {summary['missing']}")
    manifest = run.read_json(directory / "series.json")
    identities = {}
    for lane, definition in config["lanes"].items():
        _, inventory = matrix.stages(config, lane, run.REMOTE / manifest["remotes"][lane])
        identities[lane] = inventory["engines"][definition["engine"]]["artifact_sha256"]
    return collect_topology_blocks(directory, config["lanes"], identities, summary["blocks"])


def capture_slow_topology(directory):
    packet = directory.parents[1]
    replay = subprocess.check_output([sys.executable, "-B", packet / "native-import-retry.py", "replay"])
    summary = json.loads(replay)
    lanes = run.read_json(directory / "series.json")["config"]["lanes"]
    identities = {lane: spec["inventory"]["engines"][spec["engine"]]["artifact_sha256"]
                  for lane, spec in lanes.items()}
    reconciled = {attempt: rows for attempt, rows in summary["attempts"].items()
                  if (directory / "blocks" / attempt / "reconciled.json").is_file()}
    return collect_topology_blocks(directory, lanes, identities, reconciled)


def capture_dense_checkout(directory):
    packet = directory.parents[1]
    portable = packet / "evidence/dense-native-portable.tar.gz"
    result = json.loads(subprocess.check_output(
        [sys.executable, "-B", packet / "portable-native.py", "check", portable, "--scratch", packet / "evidence"]))
    if result["missing"]:
        raise ValueError(f"missing dense checkout rotations {result['missing']}")
    with tarfile.open(portable) as archive:
        prefix = directory.relative_to(SUITE.parents[1])
        manifest = run.file_json(archive, str(prefix / "series.json"))
        lanes = manifest["config"]["lanes"]
        identities = {}
        for lane, definition in lanes.items():
            inventory = run.file_json(archive, str(prefix / "staging" / lane / "inventory.json"))
            identities[lane] = inventory["engines"][definition["engine"]]["artifact_sha256"]
    if manifest != run.read_json(directory / "series.json"):
        raise ValueError("dense series manifest differs from the portable capture")
    rows = {block["attempt"]: block["raw_scores"] for block in result["blocks"].values()}
    series = collect_topology_blocks(directory, lanes, identities, rows)
    config = manifest["config"]
    for block in series["blocks"]:
        for lane, score in block["scores"].items():
            raw = rows[block["attempt"]]["measured-" + lane]
            score["commit_to_last_effect_ms"] = (raw["last_effect_us"] - raw["first_commit_us"]) / 1000
            score["last_effect_to_settlement_ms"] = (raw["last_settlement_us"] - raw["last_effect_us"]) / 1000
            if score["engine"] == "mesh":
                score.update(broker_sha256=config["mesh_broker"][lane]["elf"],
                             runner_sha256=config["mesh_runner"][lane]["elf"],
                             checkout_probes=raw["checkout_probes"], terminal_xids=raw["terminal_xids"])
    series["contrasts"] = {name: result["summary"][name] for name in ("d2", "writer")}
    return series


def capture_slow_cap71(directory):
    packet = directory.parents[2]
    result = json.loads(subprocess.check_output([
        sys.executable, "-B", packet / "sql-audit-retry.py", "replay",
        "--mode", "timing", "--directory", directory,
    ]))
    manifest = run.read_json(directory / "series.json")
    lanes = {engine: {"engine": engine} for engine in manifest["remotes"]}
    design = manifest["design"]
    identities = {engine: spec["artifact_sha256"] for engine, spec in design["stock_engines"].items()}
    identities["go"] = design["river_sha"]
    captured = {}
    for receipt in sorted((directory / "blocks").glob("*/reconciled.json")):
        attempt = receipt.parent.name
        if attempt not in result["qualified"]:
            raise ValueError(f"{receipt}: recorded slow rotation failed replay")
        captured[attempt] = result["qualified"][attempt]
    series = collect_topology_blocks(directory, lanes, identities, captured)
    fixed_first_writes = {}
    for block in series["blocks"]:
        rows = captured[block["attempt"]]
        for engine, score in block["scores"].items():
            raw = rows["measured-" + engine]
            score["committed_hold_peak"] = raw["committed_hold_peak"]
            score["physical_session_peak"] = raw["physical"]["peak"]
        path = directory / "blocks" / block["attempt"] / "measured-mesh/measured-mesh/raw.tar.gz"
        if run.digest(path) != rows["measured-mesh"]["raw_sha256"]:
            raise ValueError(f"{path}: archive changed after replay")
        with tarfile.open(path) as archive:
            jobs = run.file_json(archive, "adapter/ledger.json")["jobs"]
        earliest_finish = max(j["attempt_committed_us"] + 1000 * j["hold_ms"] for j in jobs)
        mesh_ms = (earliest_finish - rows["measured-mesh"]["first_commit_us"]) / 1000
        fixed_first_writes[block["group"]] = mesh_ms - rows["measured-rust"]["drain_ms"]
    series["fixed_first_write_finish_minus_apalis_ms"] = fixed_first_writes
    return series


def collect_topology_blocks(directory, lanes, identities, replayed):
    blocks = []
    for attempt, rows in sorted(replayed.items()):
        path = directory / "blocks" / attempt
        if rows != run.read_json(path / "reconciled.json"):
            raise ValueError(f"{path}: replay differs from controller receipt")
        block = run.read_json(path / "order.json")
        scores, archives = {}, {}
        for phase in block["phases"]:
            for lane in block["order"]:
                engine = lanes[lane]["engine"]
                row = rows[f"{phase}-{lane}"]
                archives[f"{phase}-{lane}/{phase}-{engine}"] = row["raw_sha256"]
                if phase == "measured":
                    scores[lane] = {metric: row[metric] for metric in METRICS}
                    scores[lane].update(engine=engine, completed=row["completed"], sha256=identities[lane])
        blocks.append(dict(group=block["group"], profile=block["profile"], attempt=attempt,
                           order=block["order"], scores=scores, raw_sha256=archives))
    return dict(manifest_sha256=run.digest(directory / "series.json"), blocks=blocks)


def capture_public_dense_checkout(directory):
    replay = runpy.run_path(str(directory.parents[1] / "native_report.py"))["replay"]
    _, report = replay()
    config = run.read_json(directory / "series.json")["config"]
    identities = {}
    for label, lane in config["lanes"].items():
        inventory = run.read_json(directory / "staging" / label / "inventory.json")
        identities[label] = inventory["engines"][lane["engine"]]["artifact_sha256"]
    return collect_topology_blocks(directory, config["lanes"], identities,
                                   {group["attempt"]: group["scores"] for group in report["blocks"].values()})


def capture_public_dense_individual_checkout(directory):
    retry = directory.parents[1]
    campaign = "2026-11-24-dense-individual-logger-off-retry"
    releases = {}
    for groups in ([1, 2], [3, 4, 5, 6]):
        suffix = f"{groups[0]}-{groups[-1]}"
        path = directory / f"{campaign}-groups-{suffix}.released.json"
        receipt = run.read_json(path)
        run.require((receipt["id"], receipt["group_command"], receipt["groups"], receipt["exit_status"])
                    == (campaign, "dispatch", groups, 0), f"{path}: missing successful release")
        releases[suffix] = run.digest(path)
    replay = subprocess.check_output([sys.executable, "-B", retry / "retry.py", "replay"])
    retained = directory.parent / "final-replay.json"
    run.require(replay == retained.read_bytes(), f"{retained}: differs from fresh archive replay")
    report = json.loads(replay)
    run.require(report["qualified_rotations"] == 6 and not report["missing"] and not report["failures"],
                f"{directory}: expected six clean rotations")
    manifest = run.read_json(directory / "series.json")
    config = manifest["config"]
    lanes = config["lanes"]
    inventories = {lane: run.read_json(directory / "staging" / lane / "inventory.json")
                   for lane in lanes}
    identities = {lane: inventory["engines"][lanes[lane]["engine"]]["artifact_sha256"]
                  for lane, inventory in inventories.items()}
    series = collect_topology_blocks(directory, lanes, identities,
                                     {group["attempt"]: group["scores"] for group in report["blocks"].values()})
    run.require([block["group"] for block in series["blocks"]] == list(range(1, 7)), "unexpected rotation roster")
    archives = {f"blocks/{block['attempt']}/{arm}/raw.tar.gz": digest
                for block in series["blocks"] for arm, digest in block["raw_sha256"].items()}
    run.require(len(archives) == 72 and archives == report["raw"], "arm hashes differ from native replay")
    product = config["product"]
    for block in series["blocks"]:
        group = report["blocks"][str(block["group"])]
        block["contrast"] = group["contrast"]
        for lane, score in block["scores"].items():
            raw = group["scores"]["measured-" + lane]
            score.update({key: raw[key] for key in ("application_xids", "terminal_xids")})
            if score["engine"] == "mesh":
                score.update(runner_sha256=product["manifest"]["files"]["mesh"],
                             build_manifest_sha256=product["manifest_sha256"],
                             checkout_probes=raw["checkout_probes"])
    series.update(release_sha256=releases, replay_sha256=run.digest(retained),
                  **{key: report[key] for key in ("elapsed_us", "paired_hot_minus_us", "hot_peer_wins",
                                                 "paired_hot_vs_faster_control_percent", "qualifying_groups", "advances")})
    return series


def capture_dense_fixed8_peers(directory):
    campaign = directory.parents[1]
    packet_path = campaign / "controller-packet-retry.json"
    packet = run.read_json(packet_path)
    packet_sha = run.digest(packet_path)
    releases = {}
    staging = {}
    for kind, key, groups, command in (
        ("pilot", packet["id"], [0, 1, 2], "timing_command"),
        ("peers", packet["id"] + "-peers", list(range(3, 8)), "peer_command"),
    ):
        path = directory / f"{kind}-release.json"
        release = run.read_json(path)
        run.require((release["id"], release["groups"], release["exit_status"], release["group_command"])
                    == (key, groups, 0, command), f"{path}: successful release required")
        folder, = (directory / "staging").glob(f"{key}.*")
        nonce = folder.name.rsplit(".", 1)[1]
        grant = run.read_json(directory / f"grant-{nonce}.json")
        run.require((grant["id"], grant["request_sha256"]) == (key, packet_sha),
                    f"{folder}: grant differs from the sealed request")
        releases[kind], staging[kind] = run.digest(path), folder

    sums_path = directory / "SHA256SUMS"
    seals = {}
    for line in sums_path.read_text().splitlines():
        digest, name = line.split(maxsplit=1)
        path = directory / name
        run.require(not Path(name).is_absolute() and ".." not in Path(name).parts and path not in seals
                    and run.digest(path) == digest, f"{path}: archive checksum differs")
        seals[path] = digest
    replay_path = directory / "final-replay.json"
    replay = subprocess.check_output([sys.executable, "-B", campaign / "controller.py", "replay"])
    run.require(replay == replay_path.read_bytes(), f"{replay_path}: differs from fresh archive replay")
    report = json.loads(replay)
    run.require(report["peer_eligible"] and not report["failures"] and not report["repeated"]
                and set(report["groups"]) == set(map(str, range(8))), "factorial replay is incomplete")
    run.require(len(report["raw_sha256"]) == 62
                and all(seals[directory / path] == digest for path, digest in report["raw_sha256"].items()),
                "factorial raw custody differs from archive seals")

    lanes = {"combined": "mesh", "river-2x32": "go", "oban-2x32": "oban", "apalis-2x32": "rust"}
    inventories = {label: staging["peers"] / label / "inventory.json" for label in lanes}
    artifacts = {label: run.read_json(path)["engines"][lanes[label]]["artifact_sha256"]
                 for label, path in inventories.items()}
    pilot_inventory = staging["pilot"] / "combined/inventory.json"
    run.require(pilot_inventory.read_bytes() == inventories["combined"].read_bytes(),
                "the combined pilot and fresh peer rotations used different inventories")
    blocks = []
    for block in packet["peer_schedule"]:
        group = block["group"]
        folder, = (directory / "blocks").glob(f"{run.block_prefix(block)}.*")
        scores, archives = {}, {}
        for phase in block["phases"]:
            for label in block["order"]:
                engine = lanes[label]
                arm = f"{phase}-{label}/{phase}-{engine}"
                raw = folder / arm / "raw.tar.gz"
                row = report["groups"][str(group)][phase][label]
                archives[arm] = seals[raw]
                run.require(row["raw_sha256"] == archives[arm] and row["affinity"] == [0, 1]
                            and (row["completed"], row["ordered_independent_writes"], row["failed"])
                            == (1024, 1024, 0), f"{raw}: unexpected native outcome")
                if phase == "measured":
                    score = {key: row[key] for key in METRICS}
                    score.update(engine=engine, completed=row["completed"], sha256=artifacts[label], runners=2, slots=64,
                                 terminal_xids=row["terminal_xids"])
                    if engine in ("mesh", "go"):
                        score["connections"] = dense_fixed8_connections(raw, engine, row)
                    if engine == "mesh":
                        score["runner_sha256"] = packet["elf_sha256"]
                    scores[label] = score
        blocks.append(dict(**block, attempt=folder.name, scores=scores, raw_sha256=archives))
    run.require([block["group"] for block in blocks] == list(range(3, 8))
                and sum(len(block["raw_sha256"]) for block in blocks) == 40, "expected five fresh peer rotations")
    return dict(manifest_sha256=packet_sha, release_sha256=releases, replay_sha256=run.digest(replay_path),
                archive_manifest_sha256=run.digest(sums_path), pilot_inventory_sha256=seals[pilot_inventory],
                inventory_sha256={label: seals[path] for label, path in inventories.items()},
                grant_sql_sha256=packet["candidate_sha256"], runner_sha256=packet["elf_sha256"],
                pilot_gates=report["pilot_gates"], fresh_peer_pairs=report["fresh_peer_pairs"], blocks=blocks)


def dense_fixed8_connections(raw, engine, score):
    with tarfile.open(raw) as archive:
        if engine == "go":
            samples = [json.loads(line) for line in archive.extractfile("adapter/river/measured/samples.jsonl")]
            idle = [row for row in samples if row["idle"]]
            timed = [row for row in samples if not row["idle"]]
        else:
            samples = run.file_json(archive, "adapter/raw.json")
            idle, timed = samples["idle_samples"], samples["samples"]
    run.require(idle[-1]["terminal"] == 0 and timed[-1]["terminal"] == score["completed"],
                f"{raw}: connection samples do not bracket the run")
    result = dict(last_idle=idle[-1]["connections"], timed_peak=max(row["connections"] for row in timed))
    if engine == "mesh":
        result["warmed_application_tls"] = score["physical_tls_by_role"]["compiled_caller"]
    return result


def capture_dense_fixed8_river_readiness(directory):
    campaign = directory.parents[1]
    packet_path = campaign / "controller-packet.json"
    packet = run.read_json(packet_path)
    packet_sha = run.digest(packet_path)
    source = directory / "replay-rest.json"
    replay = subprocess.check_output([sys.executable, "-B", campaign / "controller.py", "replay"])
    run.require(replay == source.read_bytes(), f"{source}: differs from fresh native replay")
    report = json.loads(replay)
    lanes = {"combined": "mesh", "river-2x32": "go", "river-cold": "go", "river-warm": "go"}
    release_hashes, grant_hashes, blocks, diagnostics = {}, {}, [], {}
    for stage, group_ids in (("setup", [0]), ("pilot", [1, 2]), ("rest", [3, 4, 5, 6])):
        release_path = directory / "controller" / f"{stage}-release.json"
        release = run.read_json(release_path)
        key = f"{packet['id']}-{stage}"
        run.require((release["id"], release["groups"], release["exit_status"]) == (key, group_ids, 0)
                    and release == report["dispositions"][stage], f"{release_path}: successful release required")
        staging, = (directory / "staging").glob(f"{stage}-*")
        grant_path = directory / f"grant-{staging.name}.json"
        grant = run.read_json(grant_path)
        run.require((grant["id"], grant["request_sha256"]) == (key, packet_sha), f"{grant_path}: grant seal differs")
        release_hashes[stage], grant_hashes[stage] = run.digest(release_path), run.digest(grant_path)
        inventories = {lane: run.read_json(staging / lane / "inventory.json") for lane in lanes}
        run.require([trial["group"] for trial in packet["schedule"][stage]] == group_ids, "unexpected release roster")
        for trial in packet["schedule"][stage]:
            folder, = (directory / "blocks").glob(f"{run.block_prefix(trial)}.*")
            rows = report["groups"][str(trial["group"])]
            run.require(run.read_json(folder / "order.json") == trial
                        and run.read_json(folder / "reconciled.json") == rows, f"{folder}: replay differs")
            scores, archives = {}, {}
            for phase in trial["phases"]:
                for lane in trial["order"]:
                    engine = lanes[lane]
                    arm = f"{phase}-{lane}/{phase}-{engine}"
                    raw = folder / arm / "raw.tar.gz"
                    row = rows[phase][lane]
                    archives[arm] = run.digest(raw)
                    with tarfile.open(raw) as archive:
                        ledger_sha = hashlib.sha256(archive.extractfile("adapter/ledger.json").read()).hexdigest()
                    run.require((archives[arm], ledger_sha) == (row["raw_sha256"], row["ledger_sha256"]),
                                f"{raw}: archive or ledger digest differs")
                    if phase == "measured":
                        scores[lane] = dense_river_readiness_score(raw.parent, trial, engine, inventories[lane], row)
                        if engine == "mesh":
                            scores[lane]["runner_sha256"] = packet["mesh_elf"]
            if stage == "setup":
                diagnostics = archives
            else:
                blocks.append(dict(**trial, attempt=folder.name, scores=scores, raw_sha256=archives))
    run.require(len(diagnostics) == 4 and sum(len(b["raw_sha256"]) for b in blocks) == 48,
                "expected four setup and 48 timing archives")
    return dict(manifest_sha256=packet_sha, release_sha256=release_hashes, grant_sha256=grant_hashes,
                replay_sha256=run.digest(source), diagnostics_raw_sha256=diagnostics,
                mesh_elf_sha256=packet["mesh_elf"], grant_sql_sha256=packet["mesh_sql"],
                river_readiness_elf_sha256=packet["river_elf"], paired=report["paired"], blocks=blocks)


def dense_river_readiness_score(folder, trial, engine, inventory, row):
    profile = trial["profile"]
    original = run.PROFILES[profile]
    run.PROFILES[profile] = (2, *original[1:])
    try:
        score = run.score_arm(folder, trial, "measured", engine, inventory)
    finally:
        run.PROFILES[profile] = original
    run.require((score["completed"], score["ordered_independent_writes"], score["failed"], score["affinity"])
                == (1024, 1024, 0, [0, 1])
                and score["submit_jobs_per_s"] == row["submit_jobs_per_s"]
                and score["commit_jobs_per_s"] == row["drain_jobs_per_s"], f"{folder}: viewer clocks or outcome differ")
    return dict({key: score[key] for key in METRICS}, **row, engine=engine, completed=score["completed"],
                sha256=inventory["engines"][engine]["artifact_sha256"],
                submit_to_last_settlement_ms=row["submit_us"] / 1000,
                commit_to_last_settlement_ms=row["drain_us"] / 1000)


def capture_public_dense_8x8(path):
    reverse = path / "reverse-continuation"
    forward_dir = path / "evidence/peers"
    reverse_dir = reverse / "evidence/peers"
    packet_path = reverse / "controller-packet.json"
    packet = run.read_json(packet_path)
    replay_path = reverse_dir / "independent-replay.json"
    replay = subprocess.check_output([sys.executable, "-B", reverse / "continuation.py", "replay"])
    run.require(replay == replay_path.read_bytes(), f"{replay_path}: differs from fresh archive replay")
    report = json.loads(replay)
    roster_path = reverse_dir / "clock-and-roster.json"
    roster = run.read_json(roster_path)
    run.require(roster["matched"] == report["matched"]
                and roster["counts"] == dict(forward=12, reverse=12, excluded_interrupted_reverse=7),
                f"{roster_path}: native custody differs")

    recovery = path.parent
    pilot_path = recovery / "evidence/pilot/replay.json"
    pilot_replay = subprocess.check_output([sys.executable, "-B", recovery / "recovery.py", "replay", "pilot"])
    run.require(pilot_replay == pilot_path.read_bytes(), f"{pilot_path}: differs from fresh pilot replay")
    pilot = json.loads(pilot_replay)
    run.require(pilot["advance_to_peers"], "the sealed pilot did not admit peers")

    lanes = {"B": "mesh", "A1": "mesh", "A2": "mesh", "river": "go", "oban": "oban", "apalis": "rust"}
    forward, = packet["retained"]
    backward, = packet["schedule"]
    run.require(forward["order"] == backward["order"][::-1], "the peer rotations are not mirrored")
    blocks, releases = [], {}
    for block, directory, status, source, receipt_sha in (
        (forward, forward_dir, 1, packet["predecessor"], roster["prior_failed_release_sha256"]),
        (backward, reverse_dir, 0, packet["id"], roster["reverse_release_sha256"]),
    ):
        release_path = directory / "controller-release.json"
        release = run.read_json(release_path)
        run.require((release["id"], release["phase"], release["exit_status"], run.digest(release_path))
                    == (source, "timing", status, receipt_sha), f"{release_path}: unexpected release")
        releases[source] = receipt_sha
        folder = directory / "blocks" / run.block_prefix(block)
        rows = report["groups"][str(block["group"])]
        run.require(run.read_json(folder / "order.json") == block
                    and run.read_json(folder / "reconciled.json") == rows, f"{folder}: native order or replay differs")
        archives, scores = {}, {}
        for phase in block["phases"]:
            for label in block["order"]:
                engine = lanes[label]
                arm = f"{phase}-{label}/{phase}-{engine}"
                raw = folder / arm / "raw.tar.gz"
                row = rows[f"{phase}-{label}"]
                archives[arm] = run.digest(raw)
                run.require(row["raw_sha256"] == archives[arm] and row["affinity"] == [0, 1]
                            and (row["completed"], row["ordered_independent_writes"], row["failed"])
                            == (1024, 1024, 0), f"{raw}: unexpected native outcome")
                if phase == "measured":
                    inventory = directory / "staging" / ("A1" if label == "A2" else label) / "inventory.json"
                    spec = run.read_json(inventory)["engines"][engine]
                    score = {key: row[key] for key in METRICS}
                    score.update(engine=engine, completed=row["completed"], sha256=spec["artifact_sha256"],
                                 submit_to_last_settlement_ms=row["submit_us"] / 1000,
                                 commit_to_last_settlement_ms=row["drain_us"] / 1000)
                    if engine == "mesh":
                        score.update(runner_sha256=packet["public_elf_sha256"], terminal_xids=row["terminal_xids"],
                                     checkout_probes=row["checkout_probes"])
                    scores[label] = score
        blocks.append(dict(**block, attempt=folder.name, release=source, scores=scores, raw_sha256=archives))
    run.require([block["group"] for block in blocks] == [1, 2]
                and len({digest for block in blocks for digest in block["raw_sha256"].values()}) == 24,
                "expected two distinct 12-arm peer rotations")
    return dict(manifest_sha256=run.digest(packet_path), release_sha256=releases,
                replay_sha256=run.digest(replay_path), custody_sha256=run.digest(roster_path),
                pilot_replay_sha256=run.digest(pilot_path), pilot_gates=pilot["gate"],
                pilot_hot_cold=roster["pilot_hot_cold"], archival_counts=roster["counts"],
                matched=report["matched"], blocks=blocks)


def capture_public_large_file_external(retry):
    directory = retry / "evidence/native"
    report_path = retry / "evidence/replay.json"
    replay = subprocess.check_output([sys.executable, "-B", retry / "retry.py", "replay"])
    run.require(replay == report_path.read_bytes(), f"{report_path}: differs from fresh archive replay")
    report = json.loads(replay)
    manifest = run.read_json(directory / "series.json")
    lanes = manifest["config"]["lanes"]
    release_path = directory / "controller-release.json"
    release = run.read_json(release_path)
    packet = retry / "controller-packet.json"
    bound = run.read_json(packet)
    run.require(release["phase"] == "timing" and release["request_sha256"] == run.digest(packet)
                and Path(release["receipt"]).parts[-len(directory.relative_to(SUITE).parts):]
                == directory.relative_to(SUITE).parts, "TIMING release does not bind this archive")
    run.require([row["group"] for row in report["groups"]] == list(range(1, 6)) and len(lanes) == 7,
                "expected five seven-lane EXTERNAL rotations")
    public = (SUITE / "results/2026-11-24-trusted-jobs-product-composition"
              / "evidence/native/timing-closure-retry/large-external.inventory.json")
    runner = run.read_json(public)["artifact"]["files"]["mesh"]
    blocks = []
    for group, plan in zip(report["groups"], manifest["plan"], strict=True):
        archives, scores = {}, {}
        for phase in plan["phases"]:
            for label in plan["order"]:
                lane = lanes[label]
                engine = lane["engine"]
                receipt = group["receipts"][phase][label]
                arm = f"{phase}-{label}/{phase}-{engine}"
                archives[arm] = receipt["raw_sha256"]
                if phase == "measured":
                    row = group["scores"][label]
                    run.require(row["raw_sha256"] == archives[arm] and row["affinity"] == [0, 1]
                                and (row["completed"], row["ordered_independent_writes"], row["failed"],
                                     receipt["app_xids"]) == (64, 64, 0, 128), "unexpected EXTERNAL outcome")
                    score = {key: row[key] for key in METRICS}
                    clock = group["clocks"][label]
                    score.update(engine=engine, completed=row["completed"], application_xids=receipt["app_xids"],
                                 terminal_xids=receipt["terminal_xids"], storage=receipt["storage"],
                                 sha256=lane["inventory"]["engines"][engine]["artifact_sha256"],
                                 submit_to_last_settlement_ms=clock["submit_ms"],
                                 commit_to_last_settlement_ms=clock["drain_ms"])
                    if engine == "mesh":
                        score.update(runner_sha256=runner, broker_sha256=(
                            runner if label == "product" else report["qualification"]["broker_sha256"]))
                    scores[label] = score
        run.require(len(archives) == 14, "expected seven warmup and seven measured arm hashes")
        blocks.append(dict(**plan, attempt=group["path"], raw_sha256=archives, scores=scores,
                           clocks=group["clocks"], duplicate_spread_ms=group["inline_spread_ms"],
                           faster_duplicate_minus_file_ms=group["faster_inline_minus_file_ms"],
                           paired_ms=group["lane_minus_file_ms"]))
    return dict(manifest_sha256=run.digest(directory / "series.json"), release_sha256=run.digest(release_path),
                replay_sha256=run.digest(report_path), configuration_sha256=bound["configuration_sha256"],
                original_release_sha256=run.digest(retry.parent / "evidence/native/controller-release.json"),
                qualification=report["qualification"], blocks=blocks)


def capture_slow_storage(packet):
    sys.path.insert(0, str(packet / "replay"))
    import native as storage_run

    config = storage_run.check()
    blocks = []
    for group, order in enumerate(config["orders"], 1):
        receipts = sorted((packet / "evidence/native").glob(f"timing-*/{group:02d}-slow/reconciled.json"))
        if not receipts:
            continue
        receipt, = receipts
        block = receipt.parent
        phases = ["warmup", "measured"]
        if run.read_json(block / "order.json") != dict(group=group, order=order, phases=phases):
            raise ValueError(f"{block}: slow storage order changed")
        sealed = run.read_json(receipt)
        scores, archives = {}, {}
        for phase in phases:
            for lane in order:
                definition = config["lanes"][lane]
                engine = definition["engine"]
                name = f"{phase}-{lane}"
                row = storage_run.score(block / name, definition, config, phase)
                if row != sealed[name]:
                    raise ValueError(f"{block}/{name}: storage or timing receipt changed")
                archives[f"{name}/{phase}-{engine}"] = row["raw_sha256"]
                if phase == "measured":
                    scores[lane] = {metric: row[metric] for metric in METRICS}
                    scores[lane].update(engine=engine, completed=row["completed"],
                                        sha256=definition["inventory"]["engines"][engine]["artifact_sha256"])
        blocks.append(dict(group=group, profile="slow", attempt=f"{block.parent.name}/{block.name}",
                           order=order, scores=scores, raw_sha256=archives))
    return dict(manifest_sha256=run.digest(packet / "frozen/native-config.json"), blocks=blocks)


def capture_public_product(directory):
    packet = directory.parents[3]
    bound_path = directory.with_suffix(".inventory.json")
    bound = run.read_json(bound_path)
    manifest = run.read_json(directory / "series.json")
    setup = directory.parents[1] / "setup/trusted-jobs-product-623c46b92cc5"
    proof = run.read_json(setup / "qualified.json")
    if (bound["artifact"] != run.read_json(setup / "mesh.json")
            or run.digest(setup / "mesh.json") != bound["manifest_sha256"]
            or bound["manifest_sha256"] != proof["manifest_sha256"]
            or proof["executable_sha256"] != bound["artifact"]["files"]["mesh"]
            or run.digest(setup / "raw.tar.gz") != proof["raw_sha256"]):
        raise ValueError(f"{directory}: timed build differs from the native fault proof")
    if manifest["config"] != bound["configuration"] or manifest["plan"] != bound["recipe"]["blocks"]:
        raise ValueError(f"{directory}: timing inventory or order differs from the frozen plan")
    paths = []
    for block in manifest["plan"]:
        receipt, = (directory / "blocks").glob(f"{run.block_prefix(block)}.*/reconciled.json")
        paths.append(receipt.parent)
    output = subprocess.check_output([sys.executable, "-B", packet / "replay/intake.py",
                                      "replay", bound_path, *paths])
    if output != (directory / "summary.json").read_bytes():
        raise ValueError(f"{directory}: raw replay differs from the controller release")
    lanes = bound["configuration"]["lanes"]
    series = dict(manifest_sha256=run.digest(directory / "series.json"),
                  product_elf_sha256=proof["executable_sha256"],
                  product_modes={name: lane["product_mode"] for name, lane in lanes.items() if lane["engine"] == "mesh"},
                  blocks=[])
    decision = run.read_json(directory.parent / "short-smaller/selection-evidence.json")
    summary = json.loads(output)
    if directory.name == "short-smaller":
        wins = {key: sum(row["product-group"][key] > row["product-bulk"][key] for row in summary["groups"])
                for key in ("submit_jobs_per_s", "commit_jobs_per_s")}
        tails = {key: {lane: summary["medians"][lane][key] for lane in ("product-bulk", "product-group")}
                 for key in ("submit_to_settlement_p99_ms", "commit_to_settlement_p99_ms")}
        use_group = min(wins.values()) >= 4 and all(row["product-group"] <= row["product-bulk"] for row in tails.values())
        if (decision["grouped_wins"] != wins or decision["median_p99_ms"] != tails
                or decision["mode"] != ("product-group" if use_group else "product-bulk")):
            raise ValueError("short selection differs from paired throughput and p99")
    elif series["product_modes"] != {"product": decision["mode"]}:
        raise ValueError(f"{directory}: selected mode changed")
    series["selected_mode"] = decision["mode"]
    for block, path, group in zip(manifest["plan"], paths, summary["groups"], strict=True):
        if group != run.read_json(path / "reconciled.json")["scores"]:
            raise ValueError(f"{path}: per-arm scores differ from raw replay")
        seals = {name.lstrip("*"): digest for digest, name in
                 (line.split(maxsplit=1) for line in (path / "SHA256SUMS").read_text().splitlines())}
        scores, archives = {}, {}
        for label in block["order"]:
            lane = lanes[label]
            engine = lane["engine"]
            for phase in block["phases"]:
                folder = f"{phase}-{label}/{phase}-{engine}"
                member = f"{folder}/raw.tar.gz"
                archives[folder] = run.digest(path / member)
                if archives[folder] != seals[member]:
                    raise ValueError(f"{path / member}: archive seal differs")
            row = group[label]
            if row["failed"] or row["completed"] != row["ordered_independent_writes"] or row["affinity"] != [0, 1]:
                raise ValueError(f"{path}/{label}: durable writes or CPU affinity differ")
            scores[label] = {key: row[key] for key in METRICS}
            scores[label].update(engine=engine, completed=row["completed"],
                                 sha256=lane["inventory"]["engines"][engine]["artifact_sha256"])
            if engine == "mesh":
                for key in ("checkout_probes", "terminal_xids", "largest_terminal_cohort"):
                    scores[label][key] = row[key]
            if engine == "rust" or block["profile"] == "slow":
                with tarfile.open(path / f"measured-{label}/measured-{engine}/raw.tar.gz") as archive:
                    jobs = run.file_json(archive, "adapter/ledger.json")["jobs"]
                    if block["profile"] == "slow":
                        events = []
                        for j in jobs:
                            start, end = j["attempt_committed_us"], j["effect_committed_us"]
                            if j["hold_ms"] != 100 or end - start < 99_000:
                                raise ValueError(f"{path}/{label}: invalid slow hold")
                            events.extend(((start, 1), (end, -1)))
                        active = peak = 0
                        for _, delta in sorted(events):
                            active += delta
                            peak = max(peak, active)
                        if active or peak > bound["recipe"]["slots"]:
                            raise ValueError(f"{path}/{label}: invalid overlapping commits")
                        series.setdefault("committed_hold_peaks", {}).setdefault(str(block["group"]), {})[label] = peak
                    if engine != "rust":
                        continue
                    last = max(jobs, key=lambda j: j["settled_us"])
                    tail = last["settled_us"] - max(j["effect_committed_us"] for j in jobs)
                    if tail >= 10_000_000:
                        samples = run.file_json(archive, "adapter/samples.json")["timed"]
                        stamps = [s["offset_s"] for s in samples
                                  if s.get("terminal") == len(jobs) - 1 and s.get("executing") == 1]
                        series.setdefault("late_apalis_settlements", []).append(dict(
                            group=block["group"], job=last["job_key"], tail_ms=tail / 1000, samples=len(stamps),
                            sampled_span_ms=1000 * (max(stamps) - min(stamps)) if stamps else None))
        series["blocks"].append(dict(group=block["group"], profile=block["profile"], order=block["order"],
                                     attempt=path.name, scores=scores, raw_sha256=archives))
    return series


def capture(directory, live=None):
    directories = [path for path in (directory, live) if path and (path / "series.json").is_file()]
    if not directories:
        raise FileNotFoundError(f"missing series manifest in {directory}")
    directory = directories[0]
    manifest = run.read_json(directory / "series.json")
    manifest_digest = run.digest(directory / "series.json")
    if any(run.digest(path / "series.json") != manifest_digest for path in directories[1:]):
        raise ValueError(f"{directory.name}: archived and live inventories differ")
    config = manifest.get("config")
    inventory = manifest.get("inventory")
    blocks = []
    for block in manifest["plan"]:
        if block["phases"][-1] != "measured":
            raise ValueError("expected a measured crossover")
        prefix = run.block_prefix(block)
        receipts = [receipt for path in directories for receipt in
                    sorted((path / "blocks").glob(f"{prefix}.*/reconciled.json"))]
        if not receipts:
            continue
        # The matrix owns attempt selection. Fail explicitly if it needs to
        # select between two completed attempts of the same group.
        if len({receipt.parent.name for receipt in receipts}) != 1:
            raise ValueError(f"{directory.name}/{prefix} has multiple reconciled attempts")
        receipt = receipts[0]
        if any(run.digest(copy) != run.digest(receipt) for copy in receipts[1:]):
            raise ValueError(f"{directory.name}/{prefix} differs in the archived and live receipts")
        if block != run.read_json(receipt.parent / "order.json"):
            raise ValueError(f"{receipt}: crossover order differs from the manifest")
        sealed = run.read_json(receipt)
        scores = sealed["scores"] if config else sealed
        if set(scores) != set(block["order"]):
            raise ValueError(f"{receipt}: measured lanes differ")

        measured, archives = {}, {}
        for phase in block["phases"]:
            for lane in block["order"]:
                if config:
                    definition = config["lanes"][lane]
                    engine = definition["engine"]
                    inputs = definition["inventory"]
                    path = Path(f"{phase}-{lane}") / f"{phase}-{engine}"
                else:
                    engine, inputs = lane, inventory
                    path = Path(f"{phase}-{engine}")
                folder = receipt.parent / path
                score = run.score_arm(folder, block, phase, engine, inputs)
                archives[str(path)] = run.digest(folder / "raw.tar.gz")
                if phase != "measured":
                    continue
                for name in (*METRICS, "completed", "failed", "ordered_independent_writes"):
                    if score[name] != scores[lane][name]:
                        raise ValueError(f"{receipt}: {lane} differs on {name}")
                if (score["failed"] or score["completed"] != score["ordered_independent_writes"]
                        or score["affinity"] != [0, 1]):
                    raise ValueError(f"{receipt}: workload or CPU mask mismatch")
                measured[lane] = {name: score[name] for name in METRICS}
                measured[lane].update(engine=engine, completed=score["completed"],
                                      sha256=inputs["engines"][engine]["artifact_sha256"])

        if config and "raw_sha256" not in sealed and block["phases"] == ["measured"]:
            expected = {f"measured-{lane}/measured-{config['lanes'][lane]['engine']}": row["raw_sha256"]
                        for lane, row in scores.items()}
        else:
            expected = sealed["raw_sha256"] if config else archives
        if expected != archives:
            raise ValueError(f"{receipt}: raw archive digest mismatch")
        blocks.append(dict(group=block["group"], profile=block["profile"],
                           attempt=receipt.parent.name, order=block["order"],
                           scores=measured, raw_sha256=archives))
    return dict(manifest_sha256=manifest_digest, blocks=blocks)


def text_json(value):
    return json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n"


def add_policy_table(lines, series, title, detail, lanes, planned=5, paired=None):
    blocks = series.get("blocks", [])
    if not blocks:
        return
    comparison = (f"| {paired} paired submit change (wins) | {paired} paired drain change (wins) |" if paired
                  else "| Candidate wins from submit | Candidate wins from commit |")
    lines.extend([f"## {title}", "", f"{len(blocks)}/{planned} matched rotations. {detail}", "",
                  f"| Lane | Submit jobs/s | Drain jobs/s {comparison}", "|---|---:|---:|---:|---:|"])
    candidate = lanes[0][0]
    for key, label in lanes:
        rates = []
        wins = []
        for metric in ("submit_jobs_per_s", "commit_jobs_per_s"):
            rates.append(f"{median(b['scores'][key][metric] for b in blocks):,.2f}")
            count = sum(b["scores"][candidate][metric] > b["scores"][key][metric] for b in blocks)
            if key == candidate:
                wins.append("—")
            elif paired:
                change = median(100 * (b["scores"][candidate][metric] / b["scores"][key][metric] - 1)
                                for b in blocks)
                wins.append(f"{change:+.2f}% ({count}/{len(blocks)})")
            else:
                wins.append(f"{count}/{len(blocks)}")
        lines.append("| " + " | ".join([label, *rates, *wins]) + " |")
    lines.append("")


def add_large_file_session(lines, series):
    title = "Public large jobs with the file-session producer"
    lines.extend([f"## {title}", ""])
    blocks = series.get("blocks", [])
    failures = ("The original SETUP export failed. The controller recovered the same native SETUP archive. "
                "The first TIMING attempt failed before the file and duplicate inline adapters could launch "
                "and yielded zero qualified rotations.")
    if not blocks:
        lines.extend([failures + " Scores are pending five qualified rotations and a matching release for the path-only retry. "
                      "[Campaign evidence](../2026-11-24-public-large-file-session/resume/README.md)", ""])
        return
    metrics = ("enqueue_ms", "submit_ms", "drain_ms")
    lane_labels = {"file": "Mesh file", "inline-a": "Mesh inline A", "inline-b": "Mesh inline B",
                   "product": "Public stock Mesh", "river": "Go River", "oban": "Elixir Oban", "apalis": "Rust Apalis"}
    windows = ("First submit to enqueue commit", "Submit to last settlement", "Enqueue commit to last settlement")
    lines.extend([f"{len(blocks)} matched rotations on CPUs 0 and 1. Each lane completed 64 jobs and committed "
                  "two ordered application writes per job. The file lane and both inline duplicates used the same "
                  "runner and broker ELFs. Public stock used the same runner ELF and its stock broker. "
                  "[Interactive results](chart.html#public-large-file-session)", "",
                  "Elapsed time in milliseconds. Cells show median (min–max).", "",
                  "| Lane | " + " | ".join(windows) + " |", "|---|---:|---:|---:|"])
    for label, name in lane_labels.items():
        columns = []
        for key in metrics:
            values = [b["clocks"][label][key] for b in blocks]
            columns.append(f"{median(values):.2f} ({min(values):.2f}–{max(values):.2f})")
        lines.append("| " + " | ".join([name, *columns]) + " |")
    lines.extend(["", "Each cell below gives the median paired difference in milliseconds and the count of rotations "
                  "where file was faster. Positive differences favor file.", "",
                  "| Reference | " + " | ".join(windows) + " |", "|---|---:|---:|---:|"])
    for label in (*list(lane_labels)[1:], "faster-duplicate"):
        comparisons = [b["faster_duplicate_minus_file_ms"] if label == "faster-duplicate" else b["paired_ms"][label]
                       for b in blocks]
        columns = []
        for key in metrics:
            values = [row[key] for row in comparisons]
            columns.append(f"{median(values):+.2f} ({sum(x > 0 for x in values)}/{len(values)})")
        lines.append("| " + " | ".join([lane_labels.get(label, "Faster inline duplicate"), *columns]) + " |")
    lines.extend(["", "Within-rotation absolute difference between the two inline controls, in milliseconds.", "",
                  "| Rotation | " + " | ".join(windows) + " |", "|---|---:|---:|---:|"])
    for block in blocks:
        spread = " | ".join(f"{block['duplicate_spread_ms'][key]:.2f}" for key in metrics)
        lines.append(f"| {block['group']} | {spread} |")
    for block in blocks:
        tail = block["scores"]["apalis"]["last_application_commit_to_last_settlement_ms"]
        if tail >= 10_000:
            lines.extend(["", f"Apalis's final settlement in rotation {block['group']} came {tail / 1000:.2f} s "
                          "after its last application commit. The measured comparison includes that rotation."])
    lines.extend(["", failures + " All scores above use the released path-only retry. "
                  "[Execution orders, per-arm clocks and evidence digests](scores.json)", ""])


def add_dense_river_readiness(lines, series):
    if not series:
        return
    add_policy_table(lines, series, "Dense fixed8 and River connection readiness",
                     "Each lane uses two 32-slot runners for 1,024 jobs on CPUs 0 and 1. Rebuilt River lazy and "
                     "prewarmed modes share an ELF. This sensitivity uses six fresh rotations. "
                     "[Interactive results](chart.html#dense-fixed8-river-readiness) · "
                     "[Original stock comparison](chart.html#dense-fixed8-logical-peers)",
                     [("river-warm", "Prewarmed Go River"), ("river-cold", "Lazy rebuilt Go River"),
                      ("river-2x32", "Stock Go River"), ("combined", "Experimental Mesh fixed8 + d388")],
                     planned=6, paired="Prewarmed River")
    submit, drain = (series["paired"][clock] for clock in ("submit_us", "drain_us"))
    lines.extend(["The sealed gate requires all six gaps to have the same sign and each absolute gap to exceed the larger "
                  "of 5% of prewarmed River elapsed time and the absolute stock versus rebuilt-lazy River gap. "
                  f"The replay reports **{submit['verdict']}** from submit and **{drain['verdict']}** from enqueue commit.", "",
                  "| Group | Prewarmed River submit lead (ms) | Submit threshold (ms) | Drain lead (ms) | Drain threshold (ms) |",
                  "|---|---:|---:|---:|---:|"])
    for a, b in zip(submit["blocks"], drain["blocks"], strict=True):
        leads = (a["mesh_us"] - a["warm_us"], a["threshold_us"], b["mesh_us"] - b["warm_us"], b["threshold_us"])
        lines.append(f"| {a['group']} | " + " | ".join(f"{value / 1000:.2f}" for value in leads) + " |")
    lines.extend(["", "Paired elapsed-time differences, median (min to max) in milliseconds.", "",
                  "| Comparison | Submit | Drain |", "|---|---:|---:|"])
    for field, label in (("cold_minus_warm_us", "Time saved by warming the same River ELF"),
                         ("stock_minus_cold_us", "Frozen stock minus rebuilt-lazy River")):
        cells = []
        for clock in (submit, drain):
            times = clock["summary"][field]
            mid, low, high = (Decimal(times[key]).quantize(Decimal(1), rounding=ROUND_HALF_UP) / 1000
                              for key in ("median", "min", "max"))
            cells.append(f"{mid:+.3f} ({low:+.3f} to {high:+.3f})")
        lines.append("| " + " | ".join([label, *cells]) + " |")
    pairs = [(b["scores"]["river-cold"]["startup"], b["scores"]["river-warm"]["startup"]) for b in series["blocks"]]
    launch = "runner_launch_to_ready_us"
    through = "runner_launch_through_settlement_us"
    ready_ms = [median(row[i][launch] for row in pairs) / 1000 for i in (0, 1)]
    ready_delta = median(warm[launch] - cold[launch] for cold, warm in pairs) / 1000
    launch_delta = median(warm[through] - cold[through] for cold, warm in pairs) / 1000
    group_five, = (b["scores"]["combined"] for b in series["blocks"] if b["group"] == 5)
    lines.extend(["", "Mesh had **41 idle total sessions**, including 32 warmed application TLS sessions. "
                  "Prewarmed River had **67 idle total sessions**, with 32 shared-pool sessions and one listener "
                  "per runner, plus the observer. Stock and rebuilt-lazy River each started with seven idle sessions.", "",
                  f"River runner launch to readiness took a median {ready_ms[0]:.1f} ms in lazy mode and "
                  f"{ready_ms[1]:.1f} ms in prewarmed mode. Warming added a paired median {ready_delta:.3f} ms to "
                  f"readiness and {launch_delta:.3f} ms from runner launch through settlement, including the common idle window. "
                  "Mesh has no matching runner-launch timestamp. The cross-engine gates use the submit and drain clocks.", "",
                  f"Group 5 remains scored. Mesh recorded {group_five['cpu_iowait_percent']:.2f}% I/O wait and "
                  f"{group_five['cpu_steal_percent']:.2f}% CPU steal in that rotation. "
                  "[Native result and independent release audit](../2026-11-25-dense-fixed8-river-readiness/NATIVE-RESULTS.md)", ""])


def write_scorecard(data):
    profiles = [("ordinary-dense", "Dense no-op"), ("ordinary", "Short"),
                ("ordinary-slow", "Slow"), ("ordinary-sparse", "Sparse"),
                ("ordinary-large", "Large · EXTERNAL")]
    lines = ["# Four native queue engines on the same two CPUs", ""]
    short = data.get("public-short-selection", {})
    if short:
        public = [("public-short-selection", "Short, 1 × 16"), ("public-dense-stock", "Dense, 8 × 8"),
                  ("public-sparse", "Sparse, 1 slot"), ("public-slow", "Slow, 1 × 64"),
                  ("public-large", "Large, 8 × 8"), ("public-dense-mixed", "Dense, Mesh 2 × 32 and peers 8 × 8"),
                  ("public-dense-equal", "Dense, all engines 2 × 32"), ("public-large-external", "Large, all engines EXTERNAL")]
        lines.extend(["## Public trusted Mesh, Go River, Elixir Oban and Rust Apalis", "",
                      "Each row gives the median jobs/s and matched wins over five Linode rotations. "
                      "All rows use one Mesh executable and the policy selected on short jobs. The trusted handler runs inside its application process. "
                      "PostgreSQL 17.11 shares two CPUs with each queue. Every handler commits its two application writes independently. Submit includes producer work. Drain starts at the first enqueue commit. "
                      "Both clocks end at the final durable settlement. [Interactive results](chart.html#public-short-selection)", "",
                      "| Workload and deployment | Clock | Mesh | River | Oban | Apalis | Mesh wins vs River / Oban / Apalis |",
                      "|---|---|---:|---:|---:|---:|---|"])
        for key, label in public:
            if key not in data:
                continue
            series = data[key]
            if series["selected_mode"] != short["selected_mode"] or series["product_elf_sha256"] != short["product_elf_sha256"]:
                raise ValueError(f"{key}: public score table mixes Mesh settings or executable")
            mesh = short["selected_mode"] if key == "public-short-selection" else "product"
            rows = [b["scores"] for b in series["blocks"]]
            for metric, clock in (("submit_jobs_per_s", "Submit"), ("commit_jobs_per_s", "Drain")):
                peers = ("river", "oban", "apalis")
                rates = " | ".join(f"{median(r[lane][metric] for r in rows):,.2f}" for lane in (mesh, *peers))
                wins = " / ".join(str(sum(r[mesh][metric] > r[peer][metric] for r in rows)) for peer in peers)
                lines.append(f"| [{label}](chart.html#{key}) | {clock} | {rates} | {wins} |")
        lines.extend(["", "Stock profiles use the original runner counts and default storage. "
                      "The additional dense and large comparisons identify the changed runner counts or PostgreSQL argument storage.", ""])
        canonical = ("public-short-selection", "public-dense-stock", "public-sparse", "public-slow", "public-large")
        if all(key in data for key in canonical):
            jobs = 0
            for key in canonical:
                selected = short["selected_mode"] if key == "public-short-selection" else "product"
                for block in data[key]["blocks"]:
                    jobs += sum(block["scores"][lane]["completed"] for lane in (selected, "river", "oban", "apalis"))
            lines.extend([f"The five stock profiles cover {jobs:,} measured jobs and {2 * jobs:,} independent application commits "
                          "for the selected Mesh policy and its three peers. Warmups, the alternate short policy, and deployment sensitivities are additional runs.", ""])
        peaks = data.get("public-slow", {}).get("committed_hold_peaks")
        if peaks:
            span = [v for group in peaks.values() for v in group.values()]
            if min(span) == max(span) == 64:
                lines.extend(["All four engines reached 64 simultaneous jobs between their first and second committed writes in every slow rotation. The canonical slow workload allowed 68 PostgreSQL connections.", ""])
        for key, label in public:
            for late in data.get(key, {}).get("late_apalis_settlements", []):
                note = (f"In {label}, Apalis group {late['group']} settled its final job, `{late['job']}`, "
                        f"{late['tail_ms'] / 1000:.2f} s after the last application write committed.")
                if late["samples"]:
                    note += (f" The observer recorded one job executing and all others terminal in {late['samples']} "
                             f"samples spanning {late['sampled_span_ms'] / 1000:.2f} s.")
                lines.extend([note + " The rates and paired wins include this run.", ""])
    add_large_file_session(lines, data.get("public-large-file-session", {}))
    external_file = data.get("public-large-file-external", {})
    add_policy_table(lines, external_file, "Public large-file producer with symmetric EXTERNAL storage",
                     "Seven lanes complete the same 64 jobs with 512 KiB payloads per rotation on CPUs 0 and 1. "
                     "Every lane uses eight 8-slot runners and EXTERNAL payload storage. Mesh verifies 4 MiB work_mem. Rates and "
                     "matched throughput changes are medians. [Interactive results](chart.html#public-large-file-external)",
                     [("file", "Mesh FILE"), ("inline-a", "Mesh inline A"), ("inline-b", "Mesh inline B"),
                      ("product", "Public stock Mesh"), ("river", "Go River"), ("oban", "Elixir Oban"),
                      ("apalis", "Rust Apalis")], paired="FILE")
    if external_file:
        rotations = external_file["blocks"]
        enqueue = [b["faster_duplicate_minus_file_ms"]["enqueue_ms"] for b in rotations]
        submit = [b["faster_duplicate_minus_file_ms"]["submit_ms"] for b in rotations]
        lines.extend([f"Against the faster same-broker inline control per clock, FILE saved a paired median "
                      f"{median(enqueue):.0f} ms to enqueue commit and {median(submit):.0f} ms from submit through "
                      f"settlement. It won {sum(v > 0 for v in enqueue)}/5 producer comparisons and "
                      f"{sum(v > 0 for v in submit)}/5 full submissions. Oban drained sooner in all five rotations. "
                      "[Durable application-write intervals](../2026-11-25-large-external-write-progress/README.md)", "",
                      "The original grant parser failed before staging. These five rotations came from the released "
                      "[retry](../2026-11-25-public-large-file-external/grant-parser-retry/RESULTS.md). "
                      "[Original failure](../2026-11-25-public-large-file-external/evidence/native/README.md)", ""])
    lines.extend(["## Original V1 and protected workloads", "",
                  "The five workloads below use the original trusted Mesh V1 runner and its protected Mesh control. Each completed rotation contains both Mesh modes and all three stock peers on the same Linode. Rates are median jobs per second. The last column counts the rotations in which trusted Mesh was faster than each peer.", ""])
    for metric, title in [("submit_jobs_per_s", "Submission through last settlement"),
                          ("commit_jobs_per_s", "First enqueue commit through last settlement")]:
        lines.extend([f"## {title}", "",
                      "| Workload | Rotations | Protected Mesh | Trusted Mesh | Oban | River | Apalis | Trusted wins vs Oban / River / Apalis |",
                      "|---|---:|---:|---:|---:|---:|---:|---|"])
        for series, label in profiles:
            blocks = data.get(series, {}).get("blocks", [])
            if not blocks:
                lines.append(f"| {label} | 0/5 | — | — | — | — | — | — |")
                continue
            scores = [block["scores"] for block in blocks]
            protected, = (k for k, s in scores[0].items() if s["engine"] == "mesh" and k != "ordinary")
            peers = [next(k for k, s in scores[0].items() if s["engine"] == engine)
                     for engine in ("oban", "go", "rust")]
            rates = " | ".join(f"{median(s[k][metric] for s in scores):,.2f}"
                               for k in (protected, "ordinary", *peers))
            wins = " / ".join(str(sum(s["ordinary"][metric] > s[k][metric] for s in scores)) for k in peers)
            lines.append(f"| {label} | {len(blocks)}/5 | {rates} | {wins} |")
        lines.append("")
    peers = [("oban", "Oban"), ("river", "River"), ("apalis", "Apalis")]
    if short:
        chosen = short["selected_mode"]
        policies = {"product-bulk": "Individual terminal commits", "product-group": "Grouped terminal commits"}
        alternate, = (lane for lane in policies if lane != chosen)
        add_policy_table(lines, short, "Public trusted runner on 128 short jobs",
                         "The same b3557273 executable and batch producer ran both Mesh policies. "
                         "The candidate win columns count paired wins for the selected policy. "
                         "The frozen rule requires grouped commits to win four pairs on each clock with no increase in median p99.",
                         [(chosen, policies[chosen] + " (selected)"), (alternate, policies[alternate]), *peers])
    dense = data.get("dense-exit-confirmed", {})
    if dense:
        add_policy_table(lines, dense, "Dense jobs with confirmed teardown and hot checkout",
                         "All ten arms use two 32-slot runners on the same two CPUs. "
                         "The hot and checked checkout arms use the same Mesh executable and bulk producer. "
                         "The separate writer comparison uses checked checkout throughout.",
                         [("d2-on", "Mesh · hot checkout"), ("d2-off-a", "Mesh · checked checkout A"),
                          ("d2-off-b", "Mesh · checked checkout B"), ("mesh-v1-2x32", "Mesh V1 · scalar producer"),
                          *((f"{key}-2x32", label) for key, label in peers)], planned=6)
        add_policy_table(lines, dense, "Dense jobs with overlapping terminal writes",
                         "The zero-hold writer settles successful jobs while the dispatcher continues. "
                         "Both writer-off controls use the same runner and bulk producer.",
                         [("r2-zero", "Mesh · zero-hold writer"), ("r2-off-a", "Mesh · writer off A"),
                          ("r2-off-b", "Mesh · writer off B")], planned=6)
        checkout = dense["contrasts"]["d2"]["meaningful_rotations"]
        writer = dense["contrasts"]["writer"]["meaningful_rotations"]
        lines.extend([f"Hot checkout cleared the prespecified threshold in {checkout}/6 rotations and the "
                      f"zero-hold writer in {writer}/6. Advancement required four. Each qualifying rotation "
                      "needed an earlier final application write and savings on each durable clock above both "
                      "5% of the faster control's elapsed time and the time difference between the two controls.", ""])
    equal_slow = data.get("slow-equal-cap71", {})
    add_policy_table(lines, equal_slow, "Slow jobs with equal runner topology and a 71-session cap",
                     "All four engines run 512 jobs with two 32-slot runners and a 100 ms hold between independent "
                     "application commits. Every engine shares two CPUs with PostgreSQL.",
                     [("mesh", "Trusted Mesh V1"), ("oban", "Oban"), ("go", "River"), ("rust", "Apalis")], planned=8)
    if equal_slow.get("blocks"):
        lines.extend(["| Engine | Peak overlapping committed holds, range | Peak physical PostgreSQL sessions, range |",
                      "|---|---:|---:|"])
        for engine, label in [("mesh", "Mesh"), ("oban", "Oban"), ("go", "River"), ("rust", "Apalis")]:
            rows = [b["scores"][engine] for b in equal_slow["blocks"]]
            cells = []
            for metric in ("committed_hold_peak", "physical_session_peak"):
                low, high = min(row[metric] for row in rows), max(row[metric] for row in rows)
                cells.append(str(low) if low == high else f"{low}–{high}")
            lines.append("| " + " | ".join([label, *cells]) + " |")
        lines.extend(["", "Mesh returns its application connection during the hold. The three peer handlers "
                      "retain theirs until the second write. Session counts cover the timed interval from submit "
                      "through settlement. [Connection accounting and execution policy]"
                      "(../2026-11-24-ordinary-slow-equal-topology-cap71/README.md)", ""])
        remaining = equal_slow["fixed_first_write_finish_minus_apalis_ms"].values()
        lines.extend([f"If Mesh kept each first application commit at its observed time and finished each job "
                      f"exactly 100 ms later, its finish time minus Apalis’s actual finish would range from "
                      f"{min(remaining):+.1f} to {max(remaining):+.1f} ms, measured from each engine’s enqueue "
                      "commit. A positive value requires earlier first writes to match Apalis on drain.", ""])
    add_policy_table(lines, data.get("slow-stock-topology", {}), "Slow jobs with two Mesh runners against stock peers",
                     "Each Mesh arm uses the same V1 bulk executable. The control copies give one runner 64 slots. "
                     "The candidate gives two runners 32 slots each. All three peers use one 64-slot runner. "
                     "Every arm shares two CPUs with PostgreSQL and has a 68-connection cap.",
                     [("two", "Mesh V1 · 2 × 32"), ("one-a", "Mesh V1 · 1 × 64 A"), ("one-b", "Mesh V1 · 1 × 64 B"),
                      *((f"{key}-stock-1x64", label) for key, label in peers)], planned=6)
    for kind, topology in (("stock", "8x8"), ("equal", "2x32")):
        layout = "eight 8-slot runners" if kind == "stock" else "two 32-slot runners"
        add_policy_table(lines, data.get(f"dense-{kind}-topology", {}),
                         f"Dense jobs against peers using {layout}",
                         "Both Mesh lanes use the same V1 runner and 64 total slots. "
                         "The peer engines share the same CPU and PostgreSQL limits.",
                         [("mesh-v1-2x32", "Mesh V1 · 2 × 32"), ("mesh-v1-8x8", "Mesh V1 · 8 × 8"),
                          *((f"{key}-{topology}", label) for key, label in peers)])
    add_policy_table(lines, data.get("ordinary-cohort", {}), "Short jobs with set-based claims and grouped successes",
                     "The candidate uses set-based admission and can settle up to eight successful jobs in one transaction. "
                     "The ordinary control uses stock admission and one terminal transaction per job.",
                     [("ordinary-success-cohort", "Trusted Mesh candidate"), ("ordinary-stock", "Trusted Mesh control"),
                      ("protected-mesh", "Protected Mesh"), *peers])
    add_policy_table(lines, data.get("bulk-slow", {}), "Slow jobs with set-based enqueue",
                     "The first two lanes use the same executable with set-based enqueue enabled or disabled. "
                     "The original V1 trusted runner and its protected control use their standard producer.",
                     [("bulk", "Trusted Mesh · batch enqueue"), ("scalar", "Same binary · scalar enqueue"),
                      ("ordinary", "Original trusted Mesh"), ("protected-mesh", "Protected Mesh"), *peers])
    for policy in ("default", "external"):
        add_policy_table(lines, data.get("slow-storage", {}), f"Slow jobs with {policy.upper()} payload storage",
                         "All four engines use one 64-slot runner and two CPUs shared with PostgreSQL. "
                         "Mesh uses set-based enqueue and individual terminal commits.",
                         [(f"bulk-{policy}", "Trusted Mesh"), *((f"{key}-{policy}", label) for key, label in peers)])
    add_policy_table(lines, data.get("bulk-v2-slow", {}), "Slow jobs with grouped successes",
                     "This pilot stopped after two opposite orders. The V2 lanes can settle up to eight successes "
                     "together. V1 keeps one terminal transaction per job. Both policies used the same 512-job "
                     "input and 100 ms hold. [Extension decision](../2026-11-23-bulk-enqueue-v2-slow/README.md)",
                     [("bulk", "V2 · batch enqueue"), ("scalar", "V2 · scalar enqueue"),
                      ("v1-bulk", "V1 · batch enqueue"), ("v1-scalar", "V1 · scalar enqueue"), *peers])
    add_policy_table(lines, data.get("public-dense-hot-checkout", {}), "Dense jobs with hot PostgreSQL checkout",
                     "All engines use two 32-slot runners. Each Mesh lane runs the same executable with grouped successes.",
                     [("hot", "Mesh hot checkout"), ("checked-a", "Checked A"), ("checked-b", "Checked B"),
                      ("oban-2x32", "Oban"), ("river-2x32", "River"), ("apalis-2x32", "Apalis")], planned=6)
    individual = data.get("public-dense-individual-hot-checkout", {})
    add_policy_table(lines, individual, "Public dense jobs with hot checkout and individual settlement",
                     "Two 32-slot runners per engine complete 1,024 jobs on CPUs 0 and 1. The three Mesh lanes share "
                     "the logger-off ELF and commit each terminal transition individually. Each job commits two ordered "
                     "application writes. Rates and paired percentage changes are medians. "
                     "[Interactive results](chart.html#public-dense-individual-hot-checkout)",
                     [("hot", "Mesh hot checkout"), ("checked-a", "Mesh checked A"), ("checked-b", "Mesh checked B"),
                      ("river-2x32", "Go River"), ("oban-2x32", "Elixir Oban"), ("apalis-2x32", "Rust Apalis")],
                     planned=6, paired="Hot")
    if individual:
        gains = individual["paired_hot_vs_faster_control_percent"]
        groups = ", ".join(map(str, individual["qualifying_groups"]))
        lines.extend([f"Against the faster checked duplicate in each rotation, hot checkout gained a median "
                      f"{gains['submit']['median']:.2f}% from submit and {gains['drain']['median']:.2f}% from enqueue commit. "
                      f"Rotations {groups} cleared the preregistered margin. "
                      "[Release receipts and raw replay](../2026-11-24-dense-individual-hot-pool/logger-off-retry/evidence/native/README.md)", ""])
    fixed8 = data.get("dense-fixed8-logical-peers", {})
    add_policy_table(lines, fixed8, "Dense experimental fixed8 and d388 against stock peers",
                     "Five fresh groups run 1,024 jobs with two 32-slot runners per engine on CPUs 0 and 1. "
                     "The public-derived experimental Mesh ELF groups successful settlements in fixed8ms mode "
                     "and uses logical-key grant SQL d388. Rates and matched changes are medians. "
                     "[Interactive results](chart.html#dense-fixed8-logical-peers)",
                     [("combined", "Mesh fixed8 + d388"), ("river-2x32", "Stock Go River"),
                      ("oban-2x32", "Stock Elixir Oban"), ("apalis-2x32", "Stock Rust Apalis")], paired="Mesh")
    if fixed8:
        river = [b["scores"]["river-2x32"]["connections"] for b in fixed8["blocks"]]
        mesh = [b["scores"]["combined"]["connections"] for b in fixed8["blocks"]]
        peaks = "–".join(map(str, (min(r["timed_peak"] for r in river), max(r["timed_peak"] for r in river))))
        lines.extend([f"Stock River opened application sessions during drain. Its sampled total rose from "
                      f"{river[0]['last_idle']} idle connections to peaks of {peaks}. Mesh started with "
                      f"{mesh[0]['warmed_application_tls']} warmed application TLS sessions and held "
                      f"{mesh[0]['timed_peak']} total connections in the timed samples. A prewarmed River comparison "
                      "is a separate sensitivity. The product ELF requires its own proof and timing.", "",
                      "The two reversed factorial pilot rotations passed their advancement gates. Their combined lane "
                      "used the same image, grant SQL and launch inventory as the five fresh peer rotations. "
                      "[Same-image pilot](../2026-11-25-dense-fixed8-logical-factorial/PILOT-RESULTS.md) · "
                      "[Peer release](../2026-11-25-dense-fixed8-logical-factorial/evidence/native-retry/peers-release.json)", ""])
    add_dense_river_readiness(lines, data.get("dense-fixed8-river-readiness", {}))
    canonical = data.get("public-dense-8x8-fixed8", {})
    add_policy_table(lines, canonical, "Canonical public dense eight-runner fixed8 and d388",
                     "Each lane runs eight 8-slot runners for 1,024 jobs on CPUs 0 and 1. Mesh B uses the "
                     "canonical public 4924b588 ELF with d388 grants, fixed8 grouped settlement and hot checkout. "
                     "A1 and A2 use hot checked-individual settlement on the same ELF. Rates and paired changes "
                     "are medians. [Interactive results](chart.html#public-dense-8x8-fixed8)",
                     [("B", "Public Mesh B"), ("A1", "Public Mesh A1"), ("A2", "Public Mesh A2"),
                      ("river", "Stock Go River"), ("oban", "Stock Elixir Oban"), ("apalis", "Stock Rust Apalis")],
                     planned=2, paired="Mesh B")
    if canonical:
        lines.extend(["B led Oban on both clocks in both rotations. River and Apalis led B on both clocks in both. "
                      "The preregistered margin qualified Apalis's lead. The River and Oban comparisons remained "
                      "inconclusive because the measured gaps fell below the duplicate-control spread in rotation 2.", "",
                      "The mirrored admission pilot used hot checked-individual duplicates A1/A2 and hot candidate B. "
                      "Pilot C kept application checkout checks on B's grouped route and shared the ELF and warmed "
                      "session count. Both pilot gates passed. The scored "
                      "forward peer block came from the 06:04 exit-1 capture. The seven arms from its interrupted "
                      "reverse block remain archived, including B's warmup flagged for overlapping apt-get. "
                      "The 06:15:10 exit-0 continuation supplied the entire fresh reverse block. "
                      "[Pilot and custody](../2026-11-25-public-dense-8x8/path-recovery/README.md) · "
                      "[Peer clock and roster audit](../2026-11-25-public-dense-8x8/path-recovery/peer-stage-recovery/reverse-continuation/evidence/peers/clock-and-roster.json) · "
                      "[Reverse continuation and retained failure](../2026-11-25-public-dense-8x8/path-recovery/peer-stage-recovery/reverse-continuation/README.md)", ""])
    add_policy_table(lines, data.get("short-fused-refill", {}), "Short jobs with combined settlement and refill",
                     "The initial two rotations use opposite orders. The three additional rotations require both fused copies to improve both clocks against their paired and public controls in each pilot rotation.",
                     [("fused-a", "Fused A"), ("fused-b", "Fused B"), ("group-a", "Grouped control A"),
                      ("group-b", "Grouped control B"), ("public-group", "Public grouped control"), *peers])
    add_policy_table(lines, data.get("short-river-readiness", {}), "Short jobs with stock and prewarmed River",
                     "Prewarmed River opened its PostgreSQL connections before submit. Cold and prewarmed River used the same executable.",
                     [("r2-d2", "Mesh hot checkout"), ("r2-off", "Mesh checked checkout"),
                      ("river", "Stock River"), ("river-cold", "River cold control"), ("river-warm", "River prewarmed"),
                      ("oban", "Stock Oban"), ("apalis", "Stock Apalis")], planned=6)
    add_policy_table(lines, data.get("exit-confirmed-hot-short", {}), "Short jobs with hot checkout and confirmed actor teardown",
                     "Both trusted runners use the same bulk producer and wait for teardown before submitting "
                     "successes for settlement. Recent pooled connections skip the checkout probe in the candidate. "
                     "Each job makes two independently committed application writes.",
                     [("r2-d2", "Trusted Mesh · hot checkout"), ("r2-off", "Trusted Mesh · checked checkout"),
                      ("protected-mesh", "Protected Mesh"), *peers])
    add_policy_table(lines, data.get("hot-short", {}), "Short jobs with fewer PostgreSQL checkout probes",
                     "V2 runners use the original producer. Hot checkout sent two probes across 256 writes, versus 256 "
                     "probes in each identical control. All writes used their own committed transactions.",
                     [("ordinary-d2", "Mesh · hot checkout"), ("ordinary-base-a", "V2 control A"),
                      ("ordinary-base-b", "V2 control B"), ("river", "River")])
    add_policy_table(lines, data.get("bulk-hot-short", {}), "Short jobs with bulk enqueue and fewer checkout probes",
                     "Both Mesh lanes share the V2 bulk producer. This experimental runner can report success before "
                     "actor teardown. The comparison below it measures the separate exit-confirmed runner.",
                     [("bulk-d2", "Mesh · bulk and hot checkout"), ("bulk", "Mesh · bulk"), ("river", "River")])
    add_policy_table(lines, data.get("bulk-hot-short-peers", {}), "Short jobs with bulk enqueue and all four engines",
                     "This run reuses the two experimental V2 builds and the stock River binary from the "
                     "three-lane series above. Both V2 runners can settle before actor teardown.",
                     [("bulk-d2", "Mesh · bulk and hot checkout"), ("bulk", "Mesh · bulk"),
                      ("protected-mesh", "Protected Mesh"), *peers])
    add_policy_table(lines, data.get("exit-confirmed-short", {}), "Short jobs with confirmed actor teardown",
                     "Zero hold sends ready completions to a writer as soon as they are available. The writer-off "
                     "control runs the same executable and also waits for actor teardown. The earlier bulk V2 "
                     "runner could settle while teardown was still running.",
                     [("writer-zero", "Mesh · zero-hold writer"), ("writer-off", "Same runner · writer off"),
                      ("bulk", "Earlier bulk V2"), *peers])
    add_policy_table(lines, data.get("bulk-short", {}), "Short jobs with both producer and runner changes",
                     "Both Mesh lanes use the V2 grouped-success policy. Only the producer's set-based enqueue switch changes.",
                     [("bulk", "Trusted Mesh · batch enqueue"), ("scalar", "Same binary · scalar enqueue"), *peers])
    add_policy_table(lines, data.get("workmem-large", {}), "Large jobs with equal PostgreSQL work_mem",
                     "All four engines use EXTERNAL argument storage and 16 MiB work_mem. "
                     "Both extra Mesh controls use the same executable at 4 MiB.",
                     [("ordinary", "Trusted Mesh · 16 MiB"), ("ordinary-4mb-a", "Same binary · 4 MiB A"),
                      ("ordinary-4mb-b", "Same binary · 4 MiB B"), ("oban-external", "Oban"),
                      ("river-external", "River"), ("apalis-external", "Apalis")])
    add_policy_table(lines, data.get("broker-once-large", {}), "Large jobs with one JSON encode per specification",
                     "The three Mesh lanes use the same V1 runner. The candidate producer retains each encoded "
                     "specification across batch boundaries. All six lanes use EXTERNAL storage and 16 MiB work_mem.",
                     [("ordinary-once", "Mesh · encode once"), ("ordinary-a", "Stock producer A"),
                      ("ordinary-b", "Stock producer B"), ("oban-external", "Oban"),
                      ("river-external", "River"), ("apalis-external", "Apalis")])
    product = data.get("trusted-product", {}).get("blocks", [])
    if product:
        lines.extend(["## Public trusted execution", "",
                      "The db5937e7 product build waits for actor teardown before settling. The earlier V2 and initial completion-writer builds could settle before teardown. "
                      "The product comparison reuses the frozen V1 producer and includes both native executable identities in the arm data. "
                      "[Teardown witness](../2026-11-23-ordinary-completion-writer/combined/README.md#actor-teardown) · [Native product qualification](../2026-11-23-ordinary-trusted-product-db5937e7/README.md)", "",
                      "| Workload | Rotations | Product submit jobs/s | V1 submit jobs/s | Product drain jobs/s | V1 drain jobs/s |", "|---|---:|---:|---:|---:|---:|"])
        for profile, label in (("short", "Short"), ("noop", "Dense")):
            subset = [b for b in product if b["profile"] == profile]
            if subset:
                rates = [median(b["scores"][lane][metric] for b in subset)
                         for metric in ("submit_jobs_per_s", "commit_jobs_per_s") for lane in ("product", "frozen-v1")]
                lines.append(f"| {label} | {len(subset)} | " + " | ".join(f"{v:.2f}" for v in rates) + " |")
        lines.append("")
    lines += [
        "Dense, short, slow and sparse use each engine's standard producer. Large uses the direct-file Mesh producer with both Mesh modes and PostgreSQL EXTERNAL argument storage on every engine. The sparse workload includes the prescribed gaps between arrivals. Every completed job has two independently committed application writes.",
        "",
        "Protected Mesh executes a fresh process and fences its two application writes. Trusted Mesh runs the handler in the application's process with its database credentials. The five-workload comparison uses the V1 binaries. The short grouped-success comparison uses its own frozen V2 inventory. [Per-rotation chart](chart.html) · [Data, binary identities and raw digests](scores.json) · [Handler entry and the remaining backlog](handler-progress.md) · [Sparse per-job response times](sparse-latency.md) · [Slow-job slot use](slow-slots.md) · [How each engine groups durable completions](../2026-11-23-native-completion-batches/README.md) · [Original four-engine baseline](../2026-11-23-baseline/README.md)", "",
    ]
    (HERE / "four-engine-scorecard.md").write_text("\n".join(lines))


def write_producer_budgets(data):
    lines = [
        "# How much can faster enqueue change the native comparison?",
        "",
        "Each column uses the Mesh and peer arms from the same rotation. The calculation holds each Mesh drain fixed at its observed time after enqueue commit. It asks whether faster submission alone could have matched the peer's full time from submission through its last settlement. A real enqueue change can also affect drain time.",
        "",
        "The three counts partition the rotations. The required time saving is the signed difference between full Mesh and peer times, in rotation order. A negative value is the observed Mesh lead. All rows use single-transaction batches and include two independently committed application writes per job.",
        "",
        "The minimum drain saving credits Mesh with free enqueue in each pair. A zero means the observed Mesh drain already fits inside its peer's full submission window. Sparse jobs have separate submissions and are shown in the per-job response tables.",
        "",
        "| Trusted Mesh workload | Peer | Already faster or tied | Faster enqueue could close gap | Faster drain also needed | Mesh enqueue, median ms | Minimum drain saving with free enqueue, median ms | Required end-to-end saving by rotation, ms |",
        "|---|---|---:|---:|---:|---:|---:|---|",
    ]
    for series, workload, lane in [
        ("public-short-selection", "Public short", data.get("public-short-selection", {}).get("selected_mode")),
        ("public-dense-stock", "Public dense · all 8 × 8", "product"),
        ("public-dense-mixed", "Public dense · Mesh 2 × 32, peers 8 × 8", "product"),
        ("public-dense-equal", "Public dense · all 2 × 32", "product"),
        ("public-slow", "Public slow · all 1 × 64", "product"),
        ("public-large", "Public large · default storage", "product"),
        ("public-large-external", "Public large · EXTERNAL", "product"),
        ("ordinary", "Short", "ordinary"), ("ordinary-slow", "Slow", "ordinary"),
        ("ordinary-large", "Large · EXTERNAL", "ordinary"), ("ordinary-dense", "Dense", "ordinary"),
        ("workmem-large", "Large · EXTERNAL and 16 MiB work_mem", "ordinary"),
        ("ordinary-cohort", "Short · grouped settlements", "ordinary-success-cohort"),
        ("bulk-slow", "Slow · batch enqueue", "bulk"), ("bulk-short", "Short · batch enqueue and grouped settlements", "bulk"),
        ("bulk-hot-short-peers", "Short · batch enqueue, grouped settlements and hot checkout", "bulk-d2"),
        ("exit-confirmed-hot-short", "Short · batch enqueue, hot checkout and confirmed teardown", "r2-d2"),
        ("slow-equal-cap71", "Slow · two 32-slot runners per engine and 71 sessions", "mesh"),
    ]:
        blocks = sorted(data.get(series, {}).get("blocks", []), key=lambda b: b["group"])
        if not blocks:
            continue
        for engine, name in [("oban", "Oban"), ("go", "River"), ("rust", "Apalis")]:
            wins = producer = 0
            enqueue, gaps, drain_savings = [], [], []
            for block in blocks:
                ours = block["scores"][lane]
                peer, = (s for s in block["scores"].values() if s["engine"] == engine)
                count = ours["completed"]
                if count != peer["completed"]:
                    raise ValueError(f"{series}: inconsistent job count")
                full = round(count * 1_000_000 / ours["submit_jobs_per_s"])
                drain = round(count * 1_000_000 / ours["commit_jobs_per_s"])
                if full - drain != round(1000 * ours["first_submit_to_commit_ms"]):
                    raise ValueError(f"{series}: inconsistent durable boundaries")
                peer_full = round(count * 1_000_000 / peer["submit_jobs_per_s"])
                wins += full <= peer_full
                producer += drain <= peer_full < full
                enqueue.append((full - drain) / 1000)
                gaps.append((full - peer_full) / 1000)
                drain_savings.append(max(0, drain - peer_full) / 1000)
            savings = ", ".join(f"{gap:+.1f}" for gap in gaps)
            lines.append(f"| {workload} | {name} | {wins} | {producer} | {len(blocks) - wins - producer} | {median(enqueue):.1f} | {median(drain_savings):.1f} | {savings} |")
    lines += ["", "Trusted Mesh runs handlers with application credentials in the shared process. Large rows labeled EXTERNAL apply the same payload storage policy to every engine. The grouped-settlement variant can commit up to eight successful terminal transitions together. [Matched runs and executable identities](scores.json) · [Interactive results](chart.html)", ""]
    (HERE / "producer-budgets.md").write_text("\n".join(lines))


def same_record(previous, current):
    if current is None:
        return False
    # Earlier exports contained the same transactions and rates with fewer latency columns.
    copied = dict(current)
    copied["scores"] = {
        lane: {k: v for k, v in metrics.items()
               if k not in RESPONSE_METRICS or k in previous["scores"].get(lane, {})}
        for lane, metrics in current["scores"].items()
    }
    return copied == previous


def write_sparse_latency(data):
    metrics = ("submit_to_settlement_p50_ms", "submit_to_settlement_p99_ms",
               "commit_to_pickup_p99_ms", "commit_to_settlement_p99_ms")
    lines = ["# Response time for sparse durable jobs", "",
             "Each arm submits 24 jobs individually. After each settlement the producer waits 250 ms before its next submission. "
             "Each job commits its two application writes independently. "
             "The pause falls outside each job’s response time. Times are milliseconds.", "",
             "Each percentile column shows the median of the per-rotation percentiles. With 24 jobs, the matrix’s nearest-rank p99 "
             "is the slowest job in that arm. The final column shows the slowest response across all rotations.", ""]
    peers = [("oban", "Oban"), ("river", "River"), ("apalis", "Apalis")]
    configurations = [
        ("public-sparse", "Selected public build", [("product", "Public trusted Mesh"), *peers]),
        ("ordinary-sparse", "Original runners", [("ordinary", "Trusted Mesh V1"), ("protected-mesh", "Protected Mesh"), *peers]),
    ]
    for series, title, lanes in configurations:
        blocks = data.get(series, {}).get("blocks", [])
        if not blocks:
            continue
        lines.extend([f"## {title}", "", f"{len(blocks)} matched Linode rotations.", "",
                      "| Engine | Submit to settle p50 | Submit to settle p99 | Commit to pickup p99 | Commit to settle p99 | Worst submit to settle |",
                      "|---|---:|---:|---:|---:|---:|"])
        for lane, label in lanes:
            values = [[b["scores"][lane][k] for b in blocks] for k in metrics]
            cells = [median(v) for v in values] + [max(values[1])]
            lines.append("| " + label + " | " + " | ".join(f"{x:.2f}" for x in cells) + " |")
        lines.extend(["", "Submit to settlement p99 in each rotation, in milliseconds.", "",
                      "| Rotation | " + " | ".join(label for _, label in lanes) + " |",
                      "|---|" + "---:|" * len(lanes)])
        for block in blocks:
            cells = [block["scores"][lane]["submit_to_settlement_p99_ms"] for lane, _ in lanes]
            lines.append(f"| {block['group']} | " + " | ".join(f"{x:.2f}" for x in cells) + " |")
        lines.append("")
    lines.extend(["[Time spent between submissions](sparse-windows.md) · [Arm identities, orders, and source digests](scores.json) · [All workload scores](four-engine-scorecard.md)", ""])
    (HERE / "sparse-latency.md").write_text("\n".join(lines))


def refresh(collect, selected=()):
    target = HERE / "scores.json"
    data = run.read_json(target) if target.exists() else {}
    if collect:
        for item in run.read_json(HERE / "series-list.json"):
            name = item["name"]
            if selected and name not in selected:
                continue
            live = SUITE / item["live"] if "live" in item else None
            archived = SUITE / item["path"]
            if item.get("public_product") and not (archived / "summary.json").is_file() and name not in data:
                continue
            if (item.get("pending") and name not in data
                    and not any(path and (path / "series.json").is_file() for path in (archived, live))):
                continue
            if item.get("public_product"):
                current = capture_public_product(archived)
            elif name == "public-large-file-session":
                previous_release = data.get(name, {}).get("release_sha256")
                command = [sys.executable, "-B", HERE / "large-file-session.py", archived]
                if previous_release:
                    command.append(previous_release)
                replay = subprocess.run(command, capture_output=True, text=True)
                if replay.returncode:
                    print(f"{name}: {replay.stderr.strip()}", file=sys.stderr)
                    continue
                current = json.loads(replay.stdout)
                if previous_release and current["release_sha256"] != previous_release:
                    print(f"{name}: previously captured TIMING release changed", file=sys.stderr)
                    continue
            elif name == "public-large-file-external":
                current = capture_public_large_file_external(archived)
            elif name == "slow-storage":
                current = capture_slow_storage(archived)
            elif name == "public-dense-hot-checkout":
                current = capture_public_dense_checkout(archived)
            elif name == "public-dense-individual-hot-checkout":
                current = capture_public_dense_individual_checkout(archived)
            elif name == "dense-fixed8-logical-peers":
                current = capture_dense_fixed8_peers(archived)
            elif name == "dense-fixed8-river-readiness":
                current = capture_dense_fixed8_river_readiness(archived)
            elif name == "public-dense-8x8-fixed8":
                current = capture_public_dense_8x8(archived)
            elif name == "dense-exit-confirmed":
                current = capture_dense_checkout(archived)
            elif name == "slow-equal-cap71":
                current = capture_slow_cap71(archived)
            elif name == "slow-stock-topology":
                current = capture_slow_topology(archived)
            elif "topology" in item:
                current = capture_topology(archived, item["topology"])
            else:
                current = capture(archived, live)
            previous = data.get(name)
            if previous:
                old = {b["attempt"]: b for b in previous["blocks"]}
                new = {b["attempt"]: b for b in current["blocks"]}
                if (previous["manifest_sha256"] != current["manifest_sha256"]
                        or any(not same_record(value, new.get(key)) for key, value in old.items())):
                    raise ValueError(f"previously captured data changed for {name}")
            current.update(title=item["title"], description=item["description"])
            data[name] = current
        target.write_text(text_json(data))
    if not data:
        raise ValueError("use --collect to replay the original ledgers")
    write_scorecard(data)
    write_producer_budgets(data)
    write_sparse_latency(data)
    embedded = json.dumps(data, sort_keys=True, separators=(",", ":"), allow_nan=False)
    template = (HERE / "view.html.in").read_text()
    if template.count("__REPLAYED_SCORES__") != 1:
        raise ValueError("expected one embedded score document")
    (HERE / "chart.html").write_text(template.replace("__REPLAYED_SCORES__", embedded.replace("<", "\\u003c")))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--collect", action="store_true", help="verify the original arm archives")
    parser.add_argument("--series", nargs="+", choices=[item["name"] for item in run.read_json(HERE / "series-list.json")],
                        help="verify only these series and regenerate the full viewer with the archived scores")
    args = parser.parse_args()
    refresh(args.collect or bool(args.series), args.series)
