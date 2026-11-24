"""Split the sparse workload duration at each durable settlement and next submission."""
import hashlib
import json
import tarfile
from pathlib import Path
from statistics import median

HERE = Path(__file__).resolve().parent


def windows(ledger, score):
    transactions = ledger["transactions"]
    assert len(transactions) == len(ledger["jobs"]) == score["completed"] == 24
    jobs = {j["job_key"]: j for j in ledger["jobs"]}
    times, waits, tails = [], [], []
    previous = None
    for transaction in transactions:
        [(key, _)] = transaction["enqueued_ids"].items()
        job = jobs.pop(key)
        submitted = transaction["submitted_us"]
        terminal = job["settled_us"]
        assert submitted <= transaction["committed_us"] <= job["effect_committed_us"] <= terminal
        if previous is not None:
            waits.append(submitted - previous)
        times.append(terminal - submitted)
        tails.append(terminal - job["effect_committed_us"])
        previous = terminal
    total = previous - transactions[0]["submitted_us"]
    assert not jobs and len(waits) == 23 and min(waits) >= 249_000
    assert total == sum(times) + sum(waits)
    assert round(24e6 / score["submit_jobs_per_s"]) == total
    assert max(times) / 1000 == score["submit_to_settlement_p99_ms"]
    return [total / 1e6, sum(times) / 1e6, sum(waits) / 1e6, median(tails) / 1000]


def series_samples(config, groups):
    root = HERE.parents[1] / config["path"] / "blocks"
    samples = {}
    for block in groups:
        for lane, score in block["scores"].items():
            key = f"measured-{lane}/measured-{score['engine']}"
            path = root / block["attempt"] / key / "raw.tar.gz"
            with path.open("rb") as packed:
                assert hashlib.file_digest(packed, "sha256").hexdigest() == block["raw_sha256"][key]
            with tarfile.open(path) as archive:
                sample = windows(json.load(archive.extractfile("adapter/ledger.json")), score)
            samples.setdefault(lane, []).append(sample)
    return samples


def render():
    configs = {s["name"]: s for s in json.loads((HERE / "series-list.json").read_text())}
    data = json.loads((HERE / "scores.json").read_text())
    peers = [("oban", "Oban"), ("river", "River"), ("apalis", "Apalis")]
    lines = ["# Time between sparse submissions", "",
             "Each arm’s full duration equals its 24 individual job response times plus the 23 intervals between a settlement and the next submit. Those intervals include 23 prescribed 250 ms pauses, totaling 5.75 seconds. The tables give the median of each measure across matched Linode rotations. Column medians can come from different rotations.", ""]
    for name, title, lanes in [
        ("public-sparse", "Selected public build", [("product", "Public trusted Mesh"), *peers]),
        ("ordinary-sparse", "Original runners", [("ordinary", "Trusted Mesh V1"), ("protected-mesh", "Protected Mesh"), *peers]),
    ]:
        if name not in data:
            continue
        groups = data[name]["blocks"]
        samples = series_samples(configs[name], groups)
        lines.extend([f"## {title}", "", f"{len(groups)} matched rotations.", "",
                      "| Engine | Full arm (s) | Sum of 24 response times (s) | Sum of 23 intervals (s) | Median second application commit to settlement (ms) |",
                      "|---|---:|---:|---:|---:|"])
        for lane, label in lanes:
            cells = [median(col) for col in zip(*samples[lane], strict=True)]
            lines.append("| " + label + " | " + " | ".join(f"{v:.3f}" for v in cells) + " |")
        lines.append("")
    lines += ["[Per-job response percentiles](sparse-latency.md) · [Archive digests and whole-arm rates](scores.json)", ""]
    return "\n".join(lines)


if __name__ == "__main__":
    (HERE / "sparse-windows.md").write_text(render())
