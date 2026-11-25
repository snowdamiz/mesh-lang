"""Decompose middle-window elapsed time using only released durable timestamps."""
import hashlib
import json
import tarfile
from bisect import bisect_left, bisect_right
from itertools import pairwise

from write_progress import EVIDENCE


def timestamps(path, digest):
    assert hashlib.sha256(path.read_bytes()).hexdigest() == digest
    with tarfile.open(path) as raw:
        jobs = json.load(raw.extractfile("adapter/ledger.json"))["jobs"]
    assert len(jobs) == len({job["id"] for job in jobs}) == 1024
    assert all(job["state"] == "completed" and job["effects"] == job["attempts"] == 1
               for job in jobs)
    assert len({job[key] for job in jobs for key in ("attempt_xid", "effect_xid")}) == 2048
    assert all(job["attempt_committed_us"] <= job["effect_committed_us"] <= job["settled_us"]
               for job in jobs)
    return jobs


def decompose(jobs):
    effects = sorted(job["effect_committed_us"] for job in jobs)[255:768]
    attempts = sorted(job["attempt_committed_us"] for job in jobs)
    settlements = sorted({job["settled_us"] for job in jobs})
    gaps = [(right - left, left, right) for left, right in pairwise(effects)]
    elapsed = effects[-1] - effects[0]
    assert len(gaps) == 512 and sum(gap for gap, _, _ in gaps) == elapsed
    longest = sorted(gaps, reverse=True)
    detail = {}
    for count in (1, 5, 10, 25, 50, 100):
        top = longest[:count]
        detail[count] = {
            "sum_us": sum(gap for gap, _, _ in top),
            "gaps_with_first_write_inside": sum(
                bisect_right(attempts, left) < bisect_left(attempts, right)
                for _, left, right in top),
            "gaps_with_terminal_inside": sum(
                bisect_right(settlements, left) < bisect_left(settlements, right)
                for _, left, right in top),
        }
    return {"middle_us": elapsed, "longest": detail}


def main():
    seals = json.loads((EVIDENCE / "summary.json").read_text())["raw_sha256_by_rotation"]
    release = json.loads((EVIDENCE / "controller-release.json").read_text())
    assert release["exit_status"] == 0
    result = {}
    for group in range(1, 6):
        result[group] = {}
        for lane in ("B", "A1", "A2", "river", "oban"):
            arm = "measured-" + lane
            digest = seals[str(group)][arm]
            path, = EVIDENCE.glob(f"blocks/{group:02}-*/{arm}/*/raw.tar.gz")
            result[group][lane] = {"sha256": digest, **decompose(timestamps(path, digest))}
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
