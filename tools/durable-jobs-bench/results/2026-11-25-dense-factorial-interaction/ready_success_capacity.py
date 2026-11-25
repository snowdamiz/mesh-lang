"""Test an immediate overflow past eight on fixed completion timelines."""
from collections import defaultdict
from hashlib import sha256
from io import BytesIO
import json
import tarfile

from warm_river_occupancy import SERIES, STAGES


def read_arm(group, lane, score):
    path, = SERIES.glob(f"blocks/{group:02}-*/measured-{lane}/*/raw.tar.gz")
    raw = path.read_bytes()
    digest = sha256(raw).hexdigest()
    assert digest == score["raw_sha256"]
    with tarfile.open(fileobj=BytesIO(raw)) as archive:
        assert json.load(archive.extractfile("status.json"))["exit_code"] == 0
        encoded = archive.extractfile("adapter/ledger.json").read()
        assert sha256(encoded).hexdigest() == score["ledger_sha256"]
        ledger = json.loads(encoded)
        if lane == "combined":
            policy = json.load(archive.extractfile("adapter/queue-policy.json"))
            assert policy["queue_policy"] == "success_cohort" and policy["largest_terminal_cohort"] == 8
    jobs = ledger["jobs"]
    tx, = ledger["transactions"]
    assert ledger["runners"] == 2 and ledger["slots"] == 64
    assert len(jobs) == len({j["job_key"] for j in jobs}) == 1024
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
    assert tx["enqueued_ids"] == {j["job_key"]: str(j["id"]) for j in jobs}
    for job in jobs:
        assert job["attempt"] == job["attempts"] == job["effects"] == 1 and job["state"] == "completed"
        assert [job[k] for k in STAGES] == sorted(job[k] for k in STAGES)
    end = max(j["settled_us"] for j in jobs)
    assert end - tx["committed_us"] == score["drain_us"] and end - tx["submitted_us"] == score["submit_us"]
    return jobs, tx, digest


def full_groups(jobs):
    cohorts = defaultdict(list)
    for job in jobs:
        cohorts[int(job["settled_xid"])].append(job)
    all_xids = [int(j[k]) for j in jobs for k in ("attempt_xid", "effect_xid", "settled_xid")]
    assert 0 < min(all_xids) <= max(all_xids) < 2**31 and max(all_xids) - min(all_xids) < 100000
    assert len({j["worker_pid"] for j in jobs}) == 2
    possible = set()
    widths = []
    for xid, members in cohorts.items():
        assert 1 <= len(members) <= 8
        assert len({(j["worker_pid"], j["settled_us"]) for j in members}) == 1
        if len(members) != 8:
            continue
        runner, stamp = members[0]["worker_pid"], members[0]["settled_us"]
        # Grant credit before exit confirmation. The client's cohort selection
        # precedes assignment of the writer's XID and its commit record.
        extras = [j["job_key"] for j in jobs if j["worker_pid"] == runner
                  and int(j["effect_xid"]) < xid and j["effect_committed_us"] <= stamp < j["settled_us"]]
        possible.update(extras)
        widths.append(len(extras))
    return dict(terminal_xids=len(cohorts), full_eights=len(widths), max_eligible_extra=max(widths),
                possible_extra_jobs=len(possible),
                wholly_eligible_later_xids=sum(all(j["job_key"] in possible for j in group)
                                               for group in cohorts.values()))


def at(jobs, t):
    return {key: sum(j[key] <= t for j in jobs)
            for key in ("entered_us", "effect_committed_us", "settled_us")}


def replay():
    for phase, groups in (("pilot", [1, 2]), ("rest", [3, 4, 5, 6])):
        release = json.loads((SERIES / f"controller/{phase}-release.json").read_bytes())
        assert release["exit_status"] == 0 and release["groups"] == groups
    scores = json.loads((SERIES / "replay-rest.json").read_bytes())["groups"]
    result = []
    for group in range(1, 7):
        arms = scores[str(group)]["measured"]
        mesh, mt, mh = read_arm(group, "combined", arms["combined"])
        river, rt, rh = read_arm(group, "river-warm", arms["river-warm"])
        assert {j["job_key"]: j["payload_sha256"] for j in mesh} == {j["job_key"]: j["payload_sha256"] for j in river}
        profile = full_groups(mesh)
        assert profile["terminal_xids"] == arms["combined"]["terminal_xids"]
        rank_times = sorted(j["effect_committed_us"] for j in river)
        points = {}
        for rank in (256, 512, 768, 1024):
            stamp = rank_times[rank - 1]
            points[rank] = {lane: at(jobs, origin["committed_us"] + stamp - rt["committed_us"])
                            for lane, jobs, origin in (("mesh", mesh, mt), ("river", river, rt))}
        result.append(dict(group=group, raw_sha256={"mesh": mh, "river": rh}, overflow=profile,
                           at_warm_river_effect_rank=points))
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2, sort_keys=True))
