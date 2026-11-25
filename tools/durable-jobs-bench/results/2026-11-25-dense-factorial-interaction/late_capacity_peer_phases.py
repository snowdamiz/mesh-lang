"""Split each durable clock and bound work before the first application XID in the released 8×8 series."""
import argparse
from hashlib import sha256
import io
import json
from pathlib import Path
import tarfile

from eight_runner_peer_gap import progress
from xid_bounds import brackets

SEAL = "059ff4a5dc76bc35a434049f50c7a6e6c454c9a174aec25ba747087407d6dce5"
ENGINES = dict(B="mesh", A1="mesh", A2="mesh", reference="mesh", river="go", oban="oban", apalis="rust")


def middle_wait(jobs):
    effects = sorted(job["effect_committed_us"] for job in jobs)
    begin, end = effects[255], effects[767]
    span = end - begin
    assert span > 0
    transactions = []
    for job in jobs:
        entered, first, second = (job[k] for k in ("entered_us", "attempt_committed_us", "effect_committed_us"))
        first_xid, second_xid = (int(job[k]) for k in ("attempt_xid", "effect_xid"))
        assert entered <= first <= second and first_xid < second_xid
        transactions.extend(((first_xid, "first", entered, first), (second_xid, "second", first, second)))
    minimum = maximum = total = 0
    for _, phase, entered, commit, earliest, latest in brackets(transactions):
        if phase == "first":
            left = max(entered, begin)
            minimum += max(0, min(earliest, end) - left)
            maximum += max(0, min(latest, end) - left)
            total += max(0, min(commit, end) - left)
    assert minimum <= maximum <= total
    return dict(span_us=span, pre_first_xid_min_job_us=minimum, pre_first_xid_max_job_us=maximum,
                entry_to_first_commit_job_us=total)


def group(block, scores):
    arms = {}
    for lane, engine in ENGINES.items():
        cell = "measured-" + lane
        raw = block / cell / ("measured-" + engine) / "raw.tar.gz"
        score = scores[cell]
        data = raw.read_bytes()
        result = progress(data, score)
        with tarfile.open(fileobj=io.BytesIO(data)) as archive:
            ledger = json.load(archive.extractfile("adapter/ledger.json"))
        assert ledger["engine"] == engine
        arms[lane] = dict(**result, middle=middle_wait(ledger["jobs"]))
    return arms


def replay(evidence):
    release = json.loads((evidence / "controller-release.json").read_bytes())
    assert release["id"] == "2026-11-25-public-dense-late-capacity-fresh-peers"
    assert release["request_sha256"] == SEAL and release["exit_status"] == 0
    scored = json.loads((evidence / "scored-before-release.json").read_bytes())["groups"]
    groups = {}
    digests = set()
    for n in range(1, 6):
        block = evidence / "blocks" / f"{n:02}-noop"
        scores = scored[str(n)]
        assert scores == json.loads((block / "reconciled.json").read_bytes())
        for phase in ("measured", "warmup"):
            for lane, engine in ENGINES.items():
                cell = phase + "-" + lane
                raw = block / cell / (phase + "-" + engine) / "raw.tar.gz"
                digest = sha256(raw.read_bytes()).hexdigest()
                assert digest == scores[cell]["raw_sha256"] and digest not in digests
                digests.add(digest)
        groups[n] = group(block, scores)
    assert len(digests) == 70
    return groups


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path)
    print(json.dumps(replay(parser.parse_args().evidence), sort_keys=True, indent=2))
