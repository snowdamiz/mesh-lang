"""Reconcile middle-window job residence and the last grant in the cold-spare pilot."""
import argparse
from hashlib import sha256
from io import BytesIO
import json
from pathlib import Path
import tarfile

RELEASE = "a00cd11383e57f84620038669b07c719402c5817aab9bc2e502234bc85a75d5c"
REPLAY = "deced5f675e36c8f52fbec36a6b9c3df0bea4c409a4095a11a87750a6d9df713"
STAGES = ("claimed_us", "entered_us", "attempt_committed_us", "effect_committed_us", "settled_us")


def read(path, expected):
    data = path.read_bytes()
    assert sha256(data).hexdigest() == expected, path
    return data


def arm(payload, score):
    with tarfile.open(fileobj=BytesIO(payload)) as archive:
        ledger = json.load(archive.extractfile("adapter/ledger.json"))
    jobs = ledger["jobs"]
    tx, = ledger["transactions"]
    assert len(jobs) == len({job["job_key"] for job in jobs}) == 1024
    assert all(job["hold_ms"] == 0 and job["attempts"] == job["effects"] == 1 for job in jobs)
    sides = [job[field] for job in jobs for field in ("attempt_xid", "effect_xid")]
    assert len(set(sides)) == 2048 and set(sides).isdisjoint(job["settled_xid"] for job in jobs)
    effects = sorted(job["effect_committed_us"] for job in jobs)
    begin, end = effects[255], effects[767]
    assert end - begin == score["middle_512_us"]
    residence = [0] * (len(STAGES) - 1)
    for job in jobs:
        times = [job[field] for field in STAGES]
        assert times == sorted(times)
        for i, (start, stop) in enumerate(zip(times, times[1:])):
            residence[i] += max(0, min(stop, end) - max(start, begin))
    last_stamp = max(job["claimed_us"] for job in jobs)
    settled = max(job["settled_us"] for job in jobs)
    tx_commit = tx["committed_us"]
    segments = [last_stamp - tx_commit, effects[-1] - last_stamp, settled - effects[-1]]
    assert sum(segments) == score["drain_us"] and settled - tx["submitted_us"] == score["submit_us"]
    return dict(middle_us=end-begin, middle_job_us=dict(zip(STAGES[1:], residence)),
                mean_entered_through_effect=(residence[1] + residence[2]) / (end - begin),
                last_stamp_from_commit_us=segments[0], last_stamp_to_final_effect_us=segments[1],
                effect_to_final_settlement_us=segments[2])


def reduce(directory):
    release = json.loads(read(directory / "controller-release.json", RELEASE))
    report = json.loads(read(directory / "controller/replay.json", REPLAY))
    assert release["exit_status"] == 0 and release["request_sha256"] == report["request_sha256"]
    assert report["complete"] and not report["errors"] and report["peer_authorized"] is False
    assert len(report["arms"]) == 2
    result = {}
    for block, phases in report["arms"].items():
        measured = phases["measured"]
        assert set(measured) == {"off-a", "off-b", "original", "on"}
        result[block] = {}
        for lane, score in measured.items():
            filename = f"{block}/measured-{lane}/measured-mesh/raw.tar.gz"
            result[block][lane] = arm(read(directory / filename, report["evidence_sha256s"][filename]), score)
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pilot_directory", type=Path)
    print(json.dumps(reduce(parser.parse_args().pilot_directory), sort_keys=True, indent=2))
