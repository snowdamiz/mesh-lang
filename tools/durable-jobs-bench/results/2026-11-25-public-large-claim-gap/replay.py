"""Locate the public large-file producer's downstream gap from reconciled Linode ledgers."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent
SERIES = HERE.parent / "2026-11-24-public-large-file-session/resume/evidence/native/timing-adapter-path-retry"
LANES = {"file", "inline-a", "inline-b", "product", "river", "oban", "apalis"}
INPUT = "9d7f130f2cc690487582f73c9bea5fbb1f2661f8e416beb2f1574a6f868cc5c1"


def read_lane(folder, lane, score):
    archive, = (folder / f"measured-{lane}").glob("*/raw.tar.gz")
    digest = sha256(archive.read_bytes()).hexdigest()
    assert digest == score["raw_sha256"]
    with tarfile.open(archive) as t:
        assert json.load(t.extractfile("status.json"))["exit_code"] == 0
        ledger = json.load(t.extractfile("adapter/ledger.json"))
    jobs = ledger["jobs"]
    txn, = ledger["transactions"]
    assert ledger["input_sha256"] == INPUT and ledger["profile"] == "large"
    assert (ledger["runners"], ledger["slots"]) == (8, 64)
    database = ledger["database"]
    assert database["version"] == "17.11" and ledger["peak_connections"] <= database["database_connection_limit"] == 96
    assert all(database[k] is True for k in ("tls", "fsync", "full_page_writes", "synchronous_commit",
                                              "track_commit_timestamp"))
    assert score["affinity"] == [0, 1] and score["failed"] == 0
    assert len(jobs) == len({j["id"] for j in jobs}) == score["completed"] == 64
    assert {j["job_key"] for j in jobs} == {f"job_{i:06d}" for i in range(64)}
    assert txn["enqueued_ids"] == {j["job_key"]: j["id"] for j in jobs}
    assert all(j["effects"] == j["attempts"] == j["attempt"] == 1 and j["state"] == "completed" for j in jobs)
    app_xids = {j[key] for j in jobs for key in ("attempt_xid", "effect_xid")}
    assert len(app_xids) == 128
    assert all(j["entered_us"] <= j["attempt_committed_us"] <= j["effect_committed_us"] <= j["settled_us"]
               for j in jobs)
    if lane in ("file", "product", "inline-a", "inline-b"):
        terminals = {j["settled_xid"] for j in jobs}
        assert len(terminals) == 64 and terminals.isdisjoint(app_xids)
        assert all(j["claimed_us"] % 1000 == 0 and j["claimed_us"] <= j["entered_us"] for j in jobs)
    start, submit = txn["committed_us"], txn["submitted_us"]
    settle = max(j["settled_us"] for j in jobs)
    assert (submit, start, settle) == (score["first_submit_us"], score["first_commit_us"],
                                        score["last_settlement_us"])
    return jobs, start, settle - start, score["host_boot_id"]


def analyze(directory, count):
    plan = json.loads((directory / "series.json").read_bytes())["plan"]
    assert len(plan) == 5 and [b["group"] for b in plan] == list(range(1, 6))
    rows = []
    for block in plan[:count]:
        group = block["group"]
        assert set(block["order"]) == LANES and block["phases"] == ["warmup", "measured"]
        receipt, = directory.glob(f"blocks/{group:02d}-large.*/reconciled.json")
        folder = receipt.parent
        assert json.loads((folder / "order.json").read_bytes()) == block
        scores = json.loads(receipt.read_bytes())["scores"]
        arms = {lane: read_lane(folder, lane, scores[lane]) for lane in block["order"]}
        assert len({arm[3] for arm in arms.values()}) == 1
        mesh, start, drain, _ = arms["file"]
        times = [j["claimed_us"] - start for j in mesh]
        last = max(times)
        peak_visible = max(sum(other["claimed_us"] <= j["claimed_us"] <= other["settled_us"]
                               for other in mesh) for j in mesh)
        for peer in ("oban", "river", "apalis"):
            peer_jobs, peer_start, cutoff, _ = arms[peer]
            assert {j["job_key"]: j["payload_sha256"] for j in mesh} == {
                j["job_key"]: j["payload_sha256"] for j in peer_jobs}
            last_xid = max(peer_jobs, key=lambda j: j["settled_us"])["settled_xid"]
            cohort = [j for j in peer_jobs if j["settled_xid"] == last_xid]
            previous = [j["settled_us"] - peer_start for j in peer_jobs if j["settled_xid"] != last_xid]
            rows.append(dict(group=group, peer=peer, drain_gap=drain - cutoff,
                             stamped_after=sum(t > cutoff for t in times),
                             possibly_after=sum(t + 999 > cutoff for t in times),
                             residual=last - cutoff, tail=drain - last, peak_visible=peak_visible,
                             last_cohort=len(cohort), previous_settlement=max(previous, default=0),
                             last_effect=max(j["effect_committed_us"] for j in peer_jobs) - peer_start,
                             peer_settlement=cutoff))
    return rows


def report(rows):
    apalis = [row for row in rows if row["peer"] == "apalis"]
    late = [row["stamped_after"] for row in apalis if row["stamped_after"] > 0]
    peak = [row["peak_visible"] for row in apalis]
    lines = ["# Public large-file claim and settlement clocks", "",
             "The comparison aligns each engine to its own durable enqueue commit. The Mesh arm reads the batch "
             "from a file and keeps individual terminal transactions. Every group checks 64 jobs and 128 distinct "
             "application transactions per measured arm.", ""]
    if late:
        lines.extend([f"In {len(late)} of {len(apalis)} rotations, {min(late)}–{max(late)} of Mesh’s 64 jobs "
                      "still lacked a claim stamp when Apalis had settled its last job.", ""])
    for row in apalis:
        if row["peer_settlement"] - row["last_effect"] > 1_000_000:
            lines.extend([f"In rotation {row['group']}, Apalis had committed all 128 application writes by "
                          f"{row['last_effect']/1000:.1f} ms. It settled {64-row['last_cohort']} jobs by "
                          f"{row['previous_settlement']/1000:.1f} ms and the last {row['last_cohort']} in one "
                          f"transaction at {row['peer_settlement']/1e6:.3f} s. All completed on their first attempt. "
                          "The archives record when the commits occurred. The cause of the delay is unresolved.", ""])
    lines.extend([f"Mesh had {min(peak)}–{max(peak)} overlapping claim-stamp-to-settlement intervals at each "
                  "rotation’s peak, across eight runners with 64 configured slots. Slot reservations and client "
                  "acknowledgements may extend beyond these database timestamps. Times in the table are milliseconds.",
             "", "| Group | Peer | Mesh drain minus peer drain | Mesh jobs stamped after peer settled "
             "| Last Mesh claim stamp minus peer settlement | Mesh last stamp to last settlement |",
             "| ---: | --- | ---: | ---: | ---: | ---: |"])
    for row in rows:
        n, m = row["stamped_after"], row["possibly_after"]
        after = str(n) if n == m else f"{n}–{m}"
        lines.append(f"| {row['group']} | {row['peer']} | {row['drain_gap']/1000:+.1f} "
                     f"| {after} | {row['residual']/1000:+.1f} | {row['tail']/1000:.1f} |")
    lines.extend(["", "The database floors each Mesh claim stamp to a millisecond before COMMIT. "
                  "The count gives lower and upper bounds when that millisecond straddles the cutoff. "
                  "A positive last-stamp difference means that, with the stamp schedule fixed, even instant "
                  "settlement at every recorded stamp would still leave Mesh behind the peer. Earlier processing "
                  "can change later grants. The ledger does not time lock ownership or client acknowledgement.", "",
                  "The original series also scores enqueue and full submit, plus both same-broker inline controls. "
                  "[Controller and native receipts](../2026-11-24-public-large-file-session/resume/adapter-path-retry/) "
                  "· [Earlier checked-cursor experiment](../2026-11-24-public-large-checked-cursor/README.md)", "",
                  "Run `python3 -B replay.py` to reproduce the table from all five reconciled blocks.", ""])
    return "\n".join(lines)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--series", type=Path, default=SERIES)
    parser.add_argument("--count", type=int, choices=range(1, 6), default=5,
                        help="replay a completed prefix during the live series")
    args = parser.parse_args()
    print(report(analyze(args.series, args.count)), end="")
