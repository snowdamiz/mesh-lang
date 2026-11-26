"""Locate historical peer crossovers for LARGE FILE. Released blocks remain independent."""
from hashlib import sha256
import json
from pathlib import Path
import sys
import tarfile

R1_SHA = "5ddc191ae55fa1b2280a2e0850307a1d843fe192027a8712a2cac744fc7b6097"
PEER_SERIES = "2026-11-25-public-large-file-external/grant-parser-retry/evidence/native"
EVENTS = ("effect_committed_us", "settled_us")


def curves(ledger, input_digest):
    tx, = ledger["transactions"]
    jobs = ledger["jobs"]
    assert ledger["input_sha256"] == input_digest
    assert len(jobs) == len(set(tx["enqueued_ids"].values())) == 64
    assert tx["enqueued_ids"] == {j["job_key"]: j["id"] for j in jobs}
    assert len({j[f] for j in jobs for f in ("effect_xid", "attempt_xid")}) == 128
    assert all(j["attempts"] == j["effects"] == 1 for j in jobs)
    return {key: sorted(j[key] - tx["committed_us"] for j in jobs) for key in EVENTS}


def compare(span, peer):
    result = {}
    for event in EVENTS:
        margin = [min(row[event][i] for row in peer) - max(row[event][i] for row in span)
                  for i in range(64)]
        misses = [i for i, value in enumerate(margin, 1) if value <= 0]
        result[event] = dict(positive_ranks=sum(v > 0 for v in margin),
                             final_contiguous_lead_starts_at=1 + max(misses, default=0) if margin[-1] > 0 else None,
                             margin_us_at_rank={i: margin[i - 1] for i in (1, 8, 16, 32, 48, 56, 64)})
    return result


def pilot_curves(path, input_digest):
    score_bytes = (path / "scored-before-release.json").read_bytes()
    assert sha256(score_bytes).hexdigest() == "2125f771fec5ef5c0e26cb6ec688555f52a48c532b7eee2546d69577e7220e6e"
    score = json.loads(score_bytes)
    assert not score["summary"]["peer_stage_earned"]
    blocks = []
    for group in score["groups"]:
        measured_span = []
        for phase, arms in group["arms"].items():
            for lane, arm in arms.items():
                raw, = path.glob(f"blocks/{group['group']:02d}-*/{phase}-{lane}/*/raw.tar.gz")
                assert sha256(raw.read_bytes()).hexdigest() == arm["raw_sha256"]
                with tarfile.open(raw) as archive:
                    ledger = json.load(archive.extractfile("adapter/ledger.json"))
                points = curves(ledger, input_digest)
                if phase == "measured" and lane in ("span-a", "span-b"):
                    measured_span.append(points)
        assert len(measured_span) == 2
        blocks.append(measured_span)
    assert len(blocks) == 2
    return blocks


def replay(raw, pilot):
    assert sha256(raw.read_bytes()).hexdigest() == R1_SHA
    with tarfile.open(raw) as archive:
        ledgers = [json.load(archive.extractfile(f"./span-16{s}/ledger.json")) for s in "ab"]
    input_digest = ledgers[0]["input_sha256"]
    span = [curves(l, input_digest) for l in ledgers]
    here = Path(__file__).resolve().parent
    history = json.loads((here / "native.json").read_bytes())["blocks"]
    peer_rows = {p: [] for p in ("river", "oban")}
    read_decode = []
    for block in sorted((here.parent / PEER_SERIES / "blocks").iterdir()):
        group = str(int(block.name.split("-")[0]))
        scores = json.loads((block / "reconciled.json").read_bytes())["scores"]
        for lane in ("file", "river", "oban"):
            path, = block.glob(f"measured-{lane}/*/raw.tar.gz")
            assert sha256(path.read_bytes()).hexdigest() == history[group][lane]["raw_sha256"]
            with tarfile.open(path) as archive:
                ledger = json.load(archive.extractfile("adapter/ledger.json"))
            points = curves(ledger, input_digest)
            tx, = ledger["transactions"]
            score = scores[lane]
            assert (tx["submitted_us"], tx["committed_us"], max(j["settled_us"] for j in ledger["jobs"])) == (
                score["first_submit_us"], score["first_commit_us"], score["last_settlement_us"])
            if lane in peer_rows:
                peer_rows[lane].append(points)
            if lane == "river":
                assert tx["submitted_us"] <= tx["read_finished_us"] <= tx["decoded_us"] <= tx["enqueue_called_us"]
                read_decode.append(tx["decoded_us"] - tx["submitted_us"])
    assert all(len(rows) == 5 for rows in peer_rows.values())
    return dict(r1_sha256=R1_SHA, input_sha256=input_digest, river_read_and_decode_us=read_decode,
                historical_fastest_minus_pilot_slower_by_rank=[
                    {peer: compare(block, rows) for peer, rows in peer_rows.items()}
                    for block in pilot_curves(pilot, input_digest)],
                historical_fastest_minus_r1_slower_by_rank={peer: compare(span, rows)
                                                             for peer, rows in peer_rows.items()})


if __name__ == "__main__":
    print(json.dumps(replay(*map(Path, sys.argv[1:])), indent=2))
