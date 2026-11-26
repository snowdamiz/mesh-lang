"""Find disjoint same-owner terminal transactions that could share one stock flush."""
import json
from collections import defaultdict

from direct4_idle_budget import blob
from direct4_peer_deadlines import ledger
from ready_success_capacity import full_groups


def price(jobs):
    coalesced = full_groups(jobs)
    owners = defaultdict(lambda: defaultdict(list))
    commits = {}
    for job in jobs:
        terminal_xid = int(job["settled_xid"])
        owners[job["worker_pid"]][terminal_xid].append(job)
        for xid_field, time_field in (("attempt_xid", "attempt_committed_us"),
                                      ("effect_xid", "effect_committed_us"),
                                      ("settled_xid", "settled_us")):
            xid, stamp = int(job[xid_field]), job[time_field]
            assert commits.setdefault(xid, stamp) == stamp
    xids = sorted(commits)
    # Any transaction with an equal or higher XID commits after this one's
    # assignment. The earliest such commit bounds the preceding flush decision.
    assignment_upper = {}
    earliest_commit = commits[xids[-1]]
    for xid in reversed(xids):
        earliest_commit = min(earliest_commit, commits[xid])
        assignment_upper[xid] = earliest_commit
    effects = sorted(job["effect_committed_us"] for job in jobs)
    schedules = {}
    for boundary in ("xid_upper", "previous_commit"):
        for lag in (0, 1000, 2000, 4000):
            pairs = []
            for pid, batches in owners.items():
                ordered = sorted(batches)
                assert [commits[xid] for xid in ordered] == sorted(commits[xid] for xid in ordered)
                consumed = set()
                for i, (lead, follow) in enumerate(zip(ordered, ordered[1:])):
                    if lead in consumed or len(batches[lead]) != 8:
                        continue
                    if boundary == "previous_commit" and not i:
                        continue
                    limit = assignment_upper[lead] if boundary == "xid_upper" else commits[ordered[i - 1]]
                    assert limit <= commits[lead]
                    extra = batches[follow]
                    if any(int(j["effect_xid"]) >= lead or j["effect_committed_us"] + lag > limit for j in extra):
                        continue
                    # Adjacent edges in a chain: taking the earliest available
                    # pair gives a maximum count of disjoint pairs.
                    consumed.update((lead, follow))
                    pairs.append(dict(owner=pid, lead_xid=lead, absorbed_xid=follow, extra_rows=len(extra),
                                      middle=effects[255] <= commits[lead] < effects[767]))
            assert len({x for p in pairs for x in (p["lead_xid"], p["absorbed_xid"])}) == 2*len(pairs)
            schedules[f"{boundary}_{lag}us"] = pairs
    return dict(**coalesced, schedules=schedules)


def replay():
    result = {}
    for block in (1, 2):
        path = f"blocks/{block:02}-noop"
        scores = json.loads(blob(f"{path}/reconciled.json"))["measured"]
        result[block] = {}
        for lane in ("service2-a", "service2-b", "service4", "direct2", "direct4"):
            score = scores[lane]
            raw = blob(f"{path}/measured-{lane}/measured-mesh/raw.tar.gz")
            jobs, _ = ledger(raw, score["raw_sha256"])
            model = price(jobs)
            assert model["terminal_xids"] == score["terminal_xids"]
            result[block][lane] = dict(raw_sha256=score["raw_sha256"], **model)
    return result


if __name__ == "__main__":
    print(json.dumps(replay(), indent=2))
