"""Replay the native ledgers and build the comparison without contacting a host."""

import argparse
import json
from pathlib import Path
from statistics import median
import sys

HERE = Path(__file__).resolve().parent
SUITE = HERE.parents[1]
sys.path.insert(0, str(SUITE))
import run


METRICS = ("submit_jobs_per_s", "commit_jobs_per_s", "first_submit_to_commit_ms",
           "commit_to_pickup_p99_ms", "runner_cpu_ms_per_job", "peak_runner_rss_bytes",
           "cpu_steal_percent", "cpu_iowait_percent")


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
        ("ordinary-cohort", "Short · grouped settlements", "ordinary-success-cohort"),
    ]:
        blocks = sorted(data[series]["blocks"], key=lambda b: b["group"])
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


def refresh(collect):
    target = HERE / "scores.json"
    data = run.read_json(target) if target.exists() else {}
    if collect:
        for item in run.read_json(HERE / "series-list.json"):
            name = item["name"]
            live = SUITE / item["live"] if "live" in item else None
            current = capture(SUITE / item["path"], live)
            previous = data.get(name)
            if previous:
                old = {b["attempt"]: b for b in previous["blocks"]}
                new = {b["attempt"]: b for b in current["blocks"]}
                if (previous["manifest_sha256"] != current["manifest_sha256"]
                        or any(new.get(key) != value for key, value in old.items())):
                    raise ValueError(f"previously captured data changed for {name}")
            current.update(title=item["title"], description=item["description"])
            data[name] = current
        target.write_text(text_json(data))
    if not data:
        raise ValueError("use --collect to replay the original ledgers")
    write_producer_budgets(data)
    embedded = json.dumps(data, sort_keys=True, separators=(",", ":"), allow_nan=False)
    template = (HERE / "view.html.in").read_text()
    if template.count("__REPLAYED_SCORES__") != 1:
        raise ValueError("expected one embedded score document")
    (HERE / "chart.html").write_text(template.replace("__REPLAYED_SCORES__", embedded.replace("<", "\\u003c")))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--collect", action="store_true", help="verify the original arm archives")
    refresh(parser.parse_args().collect)
