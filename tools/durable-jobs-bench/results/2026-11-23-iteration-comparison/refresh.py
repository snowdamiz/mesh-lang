"""Replay the native ledgers and build the comparison without contacting a host."""

import argparse
import json
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

        if config and sealed["raw_sha256"] != archives:
            raise ValueError(f"{receipt}: raw archive digest mismatch")
        blocks.append(dict(group=block["group"], profile=block["profile"],
                           attempt=receipt.parent.name, order=block["order"],
                           scores=measured, raw_sha256=archives))
    return dict(manifest_sha256=manifest_digest, blocks=blocks)


def text_json(value):
    return json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n"


def add_policy_table(lines, series, title, detail, lanes, planned=5):
    blocks = series.get("blocks", [])
    if not blocks:
        return
    lines.extend([f"## {title}", "", f"{len(blocks)}/{planned} matched rotations. {detail}", "",
                  "| Lane | Submit jobs/s | Drain jobs/s | Candidate wins from submit | Candidate wins from commit |",
                  "|---|---:|---:|---:|---:|"])
    candidate = lanes[0][0]
    for key, label in lanes:
        rates = []
        wins = []
        for metric in ("submit_jobs_per_s", "commit_jobs_per_s"):
            rates.append(f"{median(b['scores'][key][metric] for b in blocks):,.2f}")
            count = sum(b["scores"][candidate][metric] > b["scores"][key][metric] for b in blocks)
            wins.append("—" if key == candidate else f"{count}/{len(blocks)}")
        lines.append("| " + " | ".join([label, *rates, *wins]) + " |")
    lines.append("")


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
            elif name == "slow-storage":
                current = capture_slow_storage(archived)
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
