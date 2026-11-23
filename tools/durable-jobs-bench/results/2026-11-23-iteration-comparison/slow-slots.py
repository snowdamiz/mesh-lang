"""Replay slow-job slot residence from the five native set-based enqueue blocks."""
import argparse
import hashlib
import json
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent
SERIES = HERE.parent / "2026-11-23-set-based-enqueue/evidence/bulk-enqueue-v1-slow/blocks"

def ledger(directory, lane, sealed):
    raw, = directory.glob(f"measured-{lane}/*/raw.tar.gz")
    key = str(raw.parent.relative_to(directory))
    assert hashlib.sha256(raw.read_bytes()).hexdigest() == sealed["raw_sha256"][key]
    with tarfile.open(raw) as archive:
        assert json.load(archive.extractfile("status.json"))["exit_code"] == 0
        assert json.load(archive.extractfile("invocation.json"))["phase"] == "measured"
        book = json.load(archive.extractfile("adapter/ledger.json"))
    assert book["slots"] == 64 and book["profile"] == "slow" and book.get("local_smoke", False) is False
    assert sealed["scores"][lane]["affinity"] == [0,1]
    rows = book["jobs"]
    assert len(rows) == len({j["job_key"] for j in rows}) == sealed["scores"][lane]["completed"] == 512
    assert all(j["hold_ms"] == 100 and j["attempts"] == j["effects"] == j["attempt"] == 1 for j in rows)
    assert len({j[k] for j in rows for k in ("attempt_xid","effect_xid")}) == 1024
    tx, = book["transactions"]
    return tx["committed_us"], rows

def table():
    lines = ["# Slot use during the five ordinary Mesh slow-job runs", "",
             "Each native run used the original V1 trusted runner, the set-based batch producer, 64 slots, and 512 jobs with a 100 ms hold. Time runs from the PostgreSQL enqueue commit. Unoccupied time sums the free slots at each instant and divides by 64. A fully empty runner for 1 ms therefore contributes 1 ms. All time columns are milliseconds.", "",
             "| Rotation | Drain | Last claim stamp | Unoccupied through last claim | Unoccupied through drain | Claim to entry | Entry to first write | Between writes | Second write to terminal |",
             "|---|---:|---:|---:|---:|---:|---:|---:|---:|"]
    peers = []
    for group in range(1,6):
        file, = SERIES.glob(f"{group:02}-*/reconciled.json")
        sealed = json.loads(file.read_text())
        origin, jobs = ledger(file.parent,"bulk",sealed)
        last_claim = max(j["claimed_us"] for j in jobs)
        terminal = max(j["settled_us"] for j in jobs)
        last_write = max(j["effect_committed_us"] for j in jobs)
        edges = ("claimed_us","entered_us","attempt_committed_us","effect_committed_us","settled_us")
        assert all(j["claimed_us"] >= origin and [j[k] for k in edges] == sorted(j[k] for k in edges) for j in jobs)
        assert len({j["settled_xid"] for j in jobs}) == 512
        def vacancy(end, padding):
            occupied = sum(max(0,min(j["settled_us"],end)-min(j["claimed_us"]+padding,j["entered_us"])) for j in jobs)
            return (end-origin-occupied/64)/1000
        stages = [sum(j[b]-j[a] for j in jobs)/64000 for a,b in zip(edges,edges[1:])]
        drain = (terminal-origin)/1000
        assert abs(drain - vacancy(terminal,0) - sum(stages)) < 1e-8
        lines.append(f"| {group} | {drain:.1f} | {(last_claim-origin)/1000:.1f} | "
                     f"{vacancy(last_claim,0):.1f}–{vacancy(last_claim,1000):.1f} | "
                     f"{vacancy(terminal,0):.1f}–{vacancy(terminal,1000):.1f} | "
                     + " | ".join(f"{s:.1f}" for s in stages) + " |")
        row = []
        for lane in ("oban","river","apalis"):
            peer_origin, peer = ledger(file.parent,lane,sealed)
            row.append((last_write-origin-max(j["settled_us"] for j in peer)+peer_origin)/1000)
        peers.append(row)
    lines += ["", "The two ends of each unoccupied-time range use the recorded claim stamps and stamps shifted up to 1 ms later, capped at actual handler entry. The last-claim cutoff stays fixed at its recorded value in both calculations. The four residence columns use the recorded stamps and are normalized by the same 64 slots. These ledger intervals include waiting and execution. They do not assign CPU costs to the dispatcher, the database or the handlers.", "",
              "## Last application write against each peer’s final settlement", "",
              "A positive number means Mesh was still doing required application work after the peer had durably finished, measured from each arm’s own enqueue commit. With handler timings fixed, removing the final settlement delay could not close that drain gap.", "",
              "| Rotation | Oban | River | Apalis |", "|---|---:|---:|---:|"]
    lines += [f"| {i} | " + " | ".join(f"{v:+.1f}" for v in row) + " |" for i,row in enumerate(peers,1)]
    lines += ["", "Earlier admission or earlier confirmed settlement can advance later handlers, which changes their observed write timestamps. The build-free V2 comparison tests that effect with the same application calls and the same slot and connection caps. [Native scores](four-engine-scorecard.md) · [Run ledgers](../2026-11-23-set-based-enqueue/evidence/bulk-enqueue-v1-slow) · [Replay](slow-slots.py)", ""]
    return "\n".join(lines)

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    result = table()
    if args.check:
        if result != (HERE/"slow-slots.md").read_text():
            raise SystemExit("saved slow slot replay differs")
    else:
        print(result,end="")
