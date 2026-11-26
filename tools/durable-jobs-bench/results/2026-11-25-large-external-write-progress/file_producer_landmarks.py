"""Partition the durable FILE result at the last claim stamp and application effect."""
from hashlib import sha256
import json
from pathlib import Path
import sys
import tarfile


def read(path):
    return json.loads(path.read_bytes())


def spans(raw, score):
    with raw.open("rb") as capture:
        assert sha256(capture.read()).hexdigest() == score["raw_sha256"]
        capture.seek(0)
        with tarfile.open(fileobj=capture, mode="r:gz") as archive:
            jobs = json.load(archive.extractfile("adapter/ledger.json"))["jobs"]
    assert len(jobs) == len({j["job_key"] for j in jobs}) == 64
    assert all(j["state"] == "completed" and j["attempt"] == 1 for j in jobs)
    submit, enqueue = score["first_submit_us"], score["first_commit_us"]
    stamp = max(j["claimed_us"] for j in jobs)
    effect = max(j["effect_committed_us"] for j in jobs)
    finish = max(j["settled_us"] for j in jobs)
    assert finish == score["last_settlement_us"] and submit < enqueue <= stamp <= effect <= finish
    return dict(enqueue=enqueue-submit, last_claim_from_commit=stamp-enqueue,
                last_effect_after_last_claim=effect-stamp, tail=finish-effect,
                drain=finish-enqueue, submit=finish-submit)


def compare(reference, candidate):
    gain = {key: reference[key] - candidate[key] for key in reference}
    assert gain["submit"] == gain["enqueue"] + gain["drain"]
    assert gain["drain"] == gain["last_claim_from_commit"] + gain["last_effect_after_last_claim"] + gain["tail"]
    return gain


def reconcile(directory, replay_path):
    release_path = directory / "controller-release.json"
    release = read(release_path)
    assert release["id"] == "2026-11-25-public-large-canonical-string-linux-file-timing"
    assert release["exit_status"] == 0 and release["grant"] == read(directory / "grant.json")
    assert release["grant"]["request_sha256"] == "2ca2a9597d17cc29eb76cdbf84e55a1db29a9bb680967695ca22a0e34dbf5a54"
    groups = read(replay_path)["groups"]
    assert len(groups) == 5
    result = []
    for order, group in enumerate(groups, 1):
        block, = sorted((directory / "blocks").glob(f"{order:02d}-*/reconciled.json"))
        original = read(block)
        assert original == {key: group[key] for key in ("scores", "receipts", "clocks")}
        timings = {}
        for lane, score in group["scores"].items():
            raw, = (block.parent / f"measured-{lane}").glob("*/raw.tar.gz")
            timings[lane] = spans(raw, score)
        faster = min(("control-a", "control-b"), key=lambda lane: timings[lane]["submit"])
        result.append(dict(order=order, faster_submit_duplicate=faster,
                           faster_duplicate_gain_us=compare(timings[faster], timings["candidate"]),
                           exact_original_gain_us=compare(timings["exact-file"], timings["candidate"]),
                           measured_raw={lane: score["raw_sha256"] for lane, score in group["scores"].items()}))
    return dict(release_sha256=sha256(release_path.read_bytes()).hexdigest(),
                replay_sha256=sha256(replay_path.read_bytes()).hexdigest(), orders=result)


if __name__ == "__main__":
    print(json.dumps(reconcile(Path(sys.argv[1]), Path(sys.argv[2])), sort_keys=True, indent=2))
