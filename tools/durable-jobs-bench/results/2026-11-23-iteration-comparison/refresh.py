"""Replay the native ledgers and build the comparison without contacting a host."""

import argparse
import json
from pathlib import Path
from statistics import median
import subprocess
import sys

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
    lines = [
        "# Four native queue engines on the same two CPUs",
        "",
        "The five workloads below use the original trusted Mesh V1 runner and its protected Mesh control. Each completed rotation contains both Mesh modes and all three stock peers on the same Linode. Rates are median jobs per second. The last column counts the rotations in which trusted Mesh was faster than each peer.",
        "",
    ]
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
        "| Trusted Mesh workload | Peer | Already faster or tied | Faster enqueue could close gap | Faster drain also needed | Mesh enqueue, median ms | Required end-to-end saving by rotation, ms |",
        "|---|---|---:|---:|---:|---:|---|",
    ]
    for series, workload, lane in [
        ("ordinary", "Short", "ordinary"), ("ordinary-slow", "Slow", "ordinary"),
        ("ordinary-large", "Large · EXTERNAL", "ordinary"), ("ordinary-dense", "Dense", "ordinary"),
        ("workmem-large", "Large · EXTERNAL and 16 MiB work_mem", "ordinary"),
        ("ordinary-cohort", "Short · grouped settlements", "ordinary-success-cohort"),
        ("bulk-slow", "Slow · batch enqueue", "bulk"), ("bulk-short", "Short · batch enqueue and grouped settlements", "bulk"),
        ("bulk-hot-short-peers", "Short · batch enqueue, grouped settlements and hot checkout", "bulk-d2"),
        ("exit-confirmed-hot-short", "Short · batch enqueue, hot checkout and confirmed teardown", "r2-d2"),
    ]:
        blocks = sorted(data.get(series, {}).get("blocks", []), key=lambda b: b["group"])
        if not blocks:
            continue
        for engine, name in [("oban", "Oban"), ("go", "River"), ("rust", "Apalis")]:
            wins = producer = 0
            enqueue, gaps = [], []
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
            savings = ", ".join(f"{gap:+.1f}" for gap in gaps)
            lines.append(f"| {workload} | {name} | {wins} | {producer} | {len(blocks) - wins - producer} | {median(enqueue):.1f} | {savings} |")
    lines += ["", "Trusted Mesh runs handlers with application credentials in the shared process. The large series applies the same EXTERNAL payload storage policy on every engine. The grouped-settlement variant can commit up to eight successful terminal transitions together. [Matched runs and executable identities](scores.json) · [Interactive results](chart.html)", ""]
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
    blocks = data.get("ordinary-sparse", {}).get("blocks", [])
    if not blocks:
        return
    metrics = ("submit_to_settlement_p50_ms", "submit_to_settlement_p99_ms",
               "commit_to_pickup_p99_ms", "commit_to_settlement_p99_ms")
    lanes = [("ordinary", "Trusted Mesh V1"), ("protected-mesh", "Protected Mesh"), ("oban", "Oban"),
             ("river", "River"), ("apalis", "Apalis")]
    lines = ["# Response time for sparse durable jobs", "",
             f"{len(blocks)} matched rotations of 24 singleton submissions per engine. Each submission followed the "
             "preceding settlement and a 250 ms pause. Each job committed its two application writes independently. "
             "The pause falls outside each job’s response time. Times are milliseconds.", "",
             "| Engine | Submit to settle p50 | Submit to settle p99 | Commit to pickup p99 | Commit to settle p99 | Worst submit to settle |",
             "|---|---:|---:|---:|---:|---:|"]
    for lane, label in lanes:
        values = [[b["scores"][lane][k] for b in blocks] for k in metrics]
        cells = [median(v) for v in values] + [max(values[1])]
        lines.append("| " + label + " | " + " | ".join(f"{x:.2f}" for x in cells) + " |")
    lines.extend(["", "Each percentile column shows the median of the per-rotation percentiles. "
                  "With 24 jobs, the matrix’s nearest-rank p99 is the slowest job in that arm. The final column is the maximum over all verified rotations.", "",
                  "| Rotation | Trusted Mesh submit to settle p99 | Protected Mesh | Oban | River | Apalis |",
                  "|---|---:|---:|---:|---:|---:|"])
    for block in blocks:
        cells = [block["scores"][lane]["submit_to_settlement_p99_ms"] for lane, _ in lanes]
        lines.append(f"| {block['group']} | " + " | ".join(f"{x:.2f}" for x in cells) + " |")
    lines.extend(["", "[Time spent between submissions](sparse-windows.md) · [Arm identities, orders, and source digests](scores.json) · [All workload scores](four-engine-scorecard.md)", ""])
    (HERE / "sparse-latency.md").write_text("\n".join(lines))


def refresh(collect):
    target = HERE / "scores.json"
    data = run.read_json(target) if target.exists() else {}
    if collect:
        for item in run.read_json(HERE / "series-list.json"):
            name = item["name"]
            live = SUITE / item["live"] if "live" in item else None
            archived = SUITE / item["path"]
            if (item.get("pending") and name not in data
                    and not any(path and (path / "series.json").is_file() for path in (archived, live))):
                continue
            if name == "slow-storage":
                current = capture_slow_storage(archived)
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
    refresh(parser.parse_args().collect)
