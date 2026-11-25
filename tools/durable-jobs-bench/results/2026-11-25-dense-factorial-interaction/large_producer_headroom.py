"""Read interior pinned-CPU idle counters from the two released LARGE FILE series."""
from hashlib import sha256
import json
import tarfile
from pathlib import Path

RESULTS = Path(__file__).resolve().parent.parent
DEFAULT = RESULTS / "2026-11-24-public-large-file-session/resume/evidence/native"
EXTERNAL = RESULTS / "2026-11-25-public-large-file-external/grant-parser-retry/evidence"


def read(path):
    return json.loads(path.read_bytes())


def arm(group, directory):
    raw, = (directory / "blocks" / group["path"] / "measured-file").glob("measured-*/raw.tar.gz")
    score = group["scores"]["file"]
    digest = sha256(raw.read_bytes()).hexdigest()
    assert digest == score["raw_sha256"] and score["affinity"] == [0, 1]
    with tarfile.open(raw) as archive:
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
        samples = [json.loads(line) for line in archive.extractfile("host-samples.jsonl")]
        assert json.load(archive.extractfile("status.json"))["exit_code"] == 0
    tx, = ledger["transactions"]
    start, commit = tx["submitted_us"], tx["committed_us"]
    jobs = ledger["jobs"]
    assert len(jobs) == len(tx["enqueued_ids"]) == 64
    assert len({j[key] for j in jobs for key in ("attempt_xid", "effect_xid")}) == 128
    stop = max(j["settled_us"] for j in jobs)
    assert (start, commit, stop) == (score["first_submit_us"], score["first_commit_us"], score["last_settlement_us"])
    assert all(a["epoch_us"] < b["epoch_us"] and a["monotonic_ns"] < b["monotonic_ns"]
               for a, b in zip(samples, samples[1:]))
    inside = [sample for sample in samples if start < sample["epoch_us"] < commit]
    assert all(set(sample["cpu"]) == {"cpu0", "cpu1"} for sample in inside)
    # /proc/stat is read before epoch_us. The first interior timestamp witnesses that
    # the second interior read began after submit. The final read finished before commit.
    idle = None
    if len(inside) >= 3:
        first, last = inside[1], inside[-1]
        idle = {cpu: last["cpu"][cpu][3] - first["cpu"][cpu][3] for cpu in ("cpu0", "cpu1")}
        assert min(idle.values()) >= 0
    return dict(group=group["group"], raw_sha256=digest, enqueue_us=commit-start,
                submit_us=stop-start, interior_samples=len(inside), idle_ticks=idle)


def replay():
    result = {}
    for name, directory, summary, release in (
        ("default", DEFAULT / "timing-adapter-path-retry", DEFAULT / "timing-adapter-path-retry-scores.json",
         DEFAULT / "timing-adapter-path-retry-release.json"),
        ("external", EXTERNAL / "native", EXTERNAL / "replay.json", EXTERNAL / "native/controller-release.json"),
    ):
        assert read(release)["exit_status"] == 0
        groups = read(summary)["groups"]
        assert [g["group"] for g in groups] == list(range(1, 6))
        result[name] = [arm(group, directory) for group in groups]
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2))
