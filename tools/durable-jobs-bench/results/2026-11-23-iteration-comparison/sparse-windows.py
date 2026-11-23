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


def render():
    name = "ordinary-sparse"
    config = next(s for s in json.loads((HERE / "series-list.json").read_text()) if s["name"] == name)
    root = HERE.parents[1] / config["path"] / "blocks"
    groups = json.loads((HERE / "scores.json").read_text())[name]["blocks"]
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
    lines = ["# Time between sparse submissions", "",
             f"Each of the {len(groups)} rotations submitted 24 jobs individually and waited for each to settle. Each arm’s full duration equals its 24 job response times plus the 23 intervals between a settlement and the next submit. Those intervals include a prescribed 250 ms pause, or 5.75 seconds per arm. The table gives the median of each measure across rotations.", "",
             "| Engine | Full arm (s) | Sum of 24 response times (s) | Sum of 23 intervals (s) | Median second application commit to settlement (ms) |",
             "|---|---:|---:|---:|---:|"]
    for lane, label in (("ordinary", "Trusted Mesh V1"), ("protected-mesh", "Protected Mesh"), ("oban", "Oban"),
                        ("river", "River"), ("apalis", "Apalis")):
        cells = [median(col) for col in zip(*samples[lane], strict=True)]
        lines.append("| " + label + " | " + " | ".join(f"{v:.3f}" for v in cells) + " |")
    lines += ["", "Apalis finished individual jobs fastest at the median. Its longer intervals between submissions affected the whole-arm rate. River picked jobs up quickly and spent most of each response waiting for the terminal commit after its second application write. Independently reported column medians can come from different rotations. [Per-job response percentiles](sparse-latency.md) · [Archive digests and whole-arm rates](scores.json)", ""]
    return "\n".join(lines)


if __name__ == "__main__":
    (HERE / "sparse-windows.md").write_text(render())
