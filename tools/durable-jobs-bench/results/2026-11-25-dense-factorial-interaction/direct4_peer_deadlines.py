"""Locate the Direct4 workload at each historical warm River deadline."""
import hashlib
import io
import json
import tarfile
from collections import Counter, defaultdict

from direct4_idle_budget import blob
from warm_river_occupancy import SERIES, STAGES


def ledger(raw, digest, *, topology=None):
    assert hashlib.sha256(raw).hexdigest() == digest
    with tarfile.open(fileobj=io.BytesIO(raw)) as archive:
        assert json.load(archive.extractfile("status.json"))["exit_code"] == 0
        data = json.load(archive.extractfile("adapter/ledger.json"))
    if topology is not None:
        assert (data["runners"], data["slots"]) == topology
    jobs = data["jobs"]
    assert len(jobs) == len({r["job_key"] for r in jobs}) == 1024
    assert len({r[k] for r in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
    assert all(r["attempt"] == r["attempts"] == r["effects"] == 1 for r in jobs)
    assert all(sorted(r[k] for k in STAGES) == [r[k] for k in STAGES] for r in jobs)
    txn, = data["transactions"]
    return jobs, txn


def clocks(jobs, txn):
    finish = max(r["settled_us"] for r in jobs)
    return {"submit_us": finish - txn["submitted_us"], "drain_us": finish - txn["committed_us"]}


def write_phases(jobs, txn):
    effects = sorted(r["effect_committed_us"] for r in jobs)
    return dict(first_64_us=effects[63] - txn["committed_us"],
                first_256_us=effects[255] - txn["committed_us"],
                middle_us=effects[767] - effects[255],
                last_256_us=effects[-1] - effects[767],
                terminal_tail_us=max(r["settled_us"] for r in jobs) - effects[-1])


def owner_supply(jobs, begin, end, stop_field):
    # An entry witnesses receipt of its entire checked batch. Other members
    # can still be awaiting local publication or dispatch.
    received = {}
    for job in jobs:
        key = job["worker_pid"], job["claimed_us"]
        received[key] = min(received.get(key, job["entered_us"]), job["entered_us"])
    events = defaultdict(Counter)
    writes = Counter()
    for job in jobs:
        pid = job["worker_pid"]
        start = max(begin, received[pid, job["claimed_us"]])
        stop = min(end, job[stop_field])
        if start < stop:
            events[start][pid] += 1
            events[stop][pid] -= 1
        effect = job["effect_committed_us"]
        if begin < effect <= end:
            writes[effect] += 1
    owners = sorted({r["worker_pid"] for r in jobs})
    assert len(owners) == 2
    live = Counter()
    occupied_us = Counter()
    supply = [dict(wall_us=0, effects=0) for _ in range(3)]
    previous = begin
    for stamp in sorted({begin, end, *events, *writes}):
        # Credit a write to the interval ending at its commit, before applying
        # any entries or completions with the same timestamp.
        duration = stamp - previous
        occupied_us[sum(live[p] > 0 for p in owners)] += duration
        bucket = supply[sum(live[p] >= 4 for p in owners)]
        bucket["wall_us"] += duration
        bucket["effects"] += writes[stamp]
        live.update(events[stamp])
        assert all(live[p] >= 0 for p in owners)
        previous = stamp
    assert not any(live.values())
    assert sum(b["wall_us"] for b in supply) == end - begin
    assert sum(b["effects"] for b in supply) == sum(writes.values())
    return dict(both_us=occupied_us[2], neither_us=occupied_us[0], one_us=occupied_us[1],
                at_least_four_each_us=supply[2]["wall_us"], by_owners_with_four=supply)


def census(jobs, deadline):
    # Mesh claim stamps are floored to a millisecond. Include the ambiguous bin separately.
    return {
        "certainly_unstamped": sum(r["claimed_us"] > deadline for r in jobs),
        "stamp_bin_crosses_deadline": sum(r["claimed_us"] <= deadline < r["claimed_us"] + 999
                                          for r in jobs),
        "unentered": sum(r["entered_us"] > deadline for r in jobs),
        "first_write_outstanding": sum(r["attempt_committed_us"] > deadline for r in jobs),
        "second_write_outstanding": sum(r["effect_committed_us"] > deadline for r in jobs),
        "unsettled": sum(r["settled_us"] > deadline for r in jobs),
    }


def claim_room(raw, jobs):
    groups = Counter((r["worker_pid"], r["claimed_us"]) for r in jobs)
    with tarfile.open(fileobj=io.BytesIO(raw)) as archive:
        policy = json.load(archive.extractfile("adapter/queue-policy.json"))
        supply = json.load(archive.extractfile("adapter/pool-supply.json"))
        assert supply["binding"] == "direct" and supply["scheduler_workers"] == 4
        assert supply["grant"] == "claim_batch_bounded_v1"
        query, = (s for s in policy["queue_statements"] if "FROM mesh_jobs.claim_batch_bounded_v1(" in s["query"])
        for i in range(2):
            log = archive.extractfile(f"adapter/runner-{i}.log")
            report = json.loads(log.read().splitlines()[-1])
            assert report["drained"] and report["batch_contention_hints"] == 0
            assert report["queues"]["default"]["limit"] == 32
    assert max(groups.values()) == 16
    # One result per claimed job, one empty marker per unproductive call.
    assert len(groups) == query["calls"] - (query["rows"] - len(jobs))
    eligible = full = 0
    for (pid, stamp), width in groups.items():
        if width == 16:
            continue
        prior = [r for r in jobs if r["claimed_us"] < stamp]
        outstanding = [r for r in prior if r["settled_us"] >= stamp + 1000]
        nearby = sum(r["worker_pid"] == pid and stamp - 4000 < r["settled_us"] < stamp + 1000
                     for r in prior)
        # Preserve both owners' existing rows in this stamp bin.
        future = sum(r["claimed_us"] > stamp for r in jobs)
        possible = min(16, width + nearby, width + future,
                       32 - sum(r["worker_pid"] == pid for r in outstanding),
                       64 - len(outstanding) - sum(w for (other, s), w in groups.items()
                                                   if s == stamp and other != pid))
        assert possible >= width
        eligible += possible > width
        full += possible == 16
    return dict(positive_grants=len(groups), width16=sum(w == 16 for w in groups.values()),
                partials_with_room=eligible, partials_with_room_for_16=full)


def warm_river():
    peer_score = json.loads((SERIES / "replay-rest.json").read_bytes())["groups"]
    peers = {}
    for group in range(1, 7):
        peer = peer_score[str(group)]["measured"]["river-warm"]
        paths = SERIES.glob(f"blocks/{group:02}-*/measured-river-warm/*/raw.tar.gz")
        path, = (p for p in paths if hashlib.sha256(p.read_bytes()).hexdigest() == peer["raw_sha256"])
        jobs, txn = ledger(path.read_bytes(), peer["raw_sha256"])
        observed = clocks(jobs, txn)
        assert all(peer[k] == v for k, v in observed.items())
        phases = write_phases(jobs, txn)
        assert phases["middle_us"] == peer["progress"]["second_write"]["middle_512_us"]
        peers[group] = dict(**observed, phases=phases, raw_sha256=peer["raw_sha256"])
    assert len(peers) == 6
    return peers


def replay():
    peers = warm_river()
    result = {}
    for group in (1, 2):
        block = f"blocks/{group:02}-noop"
        score = json.loads(blob(f"{block}/reconciled.json"))["measured"]["direct4"]
        raw = blob(f"{block}/measured-direct4/measured-mesh/raw.tar.gz")
        jobs, txn = ledger(raw, score["raw_sha256"])
        own = clocks(jobs, txn)
        assert all(score[k] == v for k, v in own.items())
        assert all(r["claimed_us"] % 1000 == 0 for r in jobs)
        enqueue = own["submit_us"] - own["drain_us"]
        phases = write_phases(jobs, txn)
        assert phases["middle_us"] == score["middle_512_us"]
        effects = sorted(r["effect_committed_us"] for r in jobs)
        supply = {}
        for phase, begin, end in (("middle", effects[255], effects[767]),
                                  ("last_256", effects[767], effects[-1])):
            supply[phase] = {field: owner_supply(jobs, begin, end, field)
                             for field in ("entered_us", "effect_committed_us")}
        comparisons = []
        for peer_group, peer in peers.items():
            # At current producer time both clocks bind. If enqueue matches River,
            # reaching River drain exactly suffices for both clocks.
            for policy, cutoff in (("current_enqueue", min(peer["drain_us"], peer["submit_us"] - enqueue)),
                                   ("matched_enqueue", peer["drain_us"])):
                deadline = txn["committed_us"] + cutoff
                middle_saving = phases["middle_us"] - peer["phases"]["middle_us"]
                comparisons.append(dict(peer_group=peer_group, policy=policy, cutoff_us=cutoff,
                                        drain_advance_us=own["drain_us"] - cutoff,
                                        remaining_if_peer_middle_us=own["drain_us"] - cutoff - middle_saving,
                                        **census(jobs, deadline)))
        result[group] = dict(**own, enqueue_us=enqueue, raw_sha256=score["raw_sha256"],
                             phases=phases, checked_batch_supply=supply,
                             claim_room=claim_room(raw, jobs), comparisons=comparisons)
    return dict(warm_river=peers, direct4=result)


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2))
