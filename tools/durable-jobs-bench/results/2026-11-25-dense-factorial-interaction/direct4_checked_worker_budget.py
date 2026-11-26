"""Price recorded Direct4 handler residence against the warm River middle interval."""
import argparse
import hashlib
import json
import runpy
import tarfile
from pathlib import Path

from direct4_peer_deadlines import warm_river


def checked_worker_budget(evidence, decoder):
    release = evidence / "controller/release.json"
    arm = evidence / "blocks/01-noop/warmup-on/warmup-mesh"
    raw = arm / "raw.tar.gz"
    for path, expected in (
        (release, "65115b81c78279e68594eaf98346ab56e8e057ff0232c1e3f1aef89ab5e6b3f8"),
        (raw, "ec73769de866fa4e808464003ac3a9f44bbfca937133a5552afe05f7c881badc"),
        (decoder, "6891d64914ff43fbabd76c5762349395a0f487b55dda8ccdb5e09f9e1b5ae91e"),
    ):
        assert hashlib.sha256(path.read_bytes()).hexdigest() == expected
    assert json.loads(release.read_bytes())["exit_status"] == 1
    assert json.loads((arm / "dispatch.json").read_bytes())["exit_code"] == 0
    decode = runpy.run_path(str(decoder))
    with tarfile.open(raw) as archive:
        def adapter(name):
            return archive.extractfile("adapter/" + name).read()

        ledger = json.loads(adapter("ledger.json"))
        assert (ledger["phase"], ledger["slots"], ledger["runners"]) == ("warmup", 64, 2)
        broker, *runners = json.loads(adapter("process-identity.json"))
        assert len(runners) == 2
        assert all(proc["executable_sha256"] == "806f8022a0755a62661e9f05a8c91709a9b4147872c2616781fd835626dbb06e"
                   for proc in (broker, *runners))
        trace = {}
        for i, proc in enumerate(runners):
            pid = proc["pid"]
            logs = (adapter(f"runner-{i}.trace-{pid}-{lane}.log") for lane in (0, 1, 2, 3, 255))
            trace[pid] = decode["frames"](b"".join(logs), pid, True)
    txn, = ledger["transactions"]
    jobs, _, windows, alignment = decode["join"](ledger["jobs"], trace, txn)
    assert len(jobs) == len({job["id"] for job in jobs}) == 1024
    begin, end = windows["middle_512"]
    slack = alignment["alignment_allowance_ns"]

    def duration(start, stop):
        return max(0, min(end, stop) - max(begin, start))

    peers = warm_river()
    slowest = max(peer["phases"]["middle_us"] for peer in peers.values()) * 1000
    owners = []
    for pid in sorted(trace):
        own = [job for job in jobs if job["pid"] == pid]
        def area(start, stop):
            return sum(duration(job[start], job[stop]) for job in own)

        handler = area("start", "release")
        assert handler <= 4 * (end - begin)
        replies = []
        for n in (1, 2):
            sent, ready, record = f"sent{n}", f"ready{n}", f"record{n}"
            total = area(sent, ready)
            post = sum(duration(max(job[sent], job[record] + slack), job[ready]) for job in own)
            assert 0 <= post <= total
            replies.append(dict(send_to_ready_ns=total, definitely_after_commit_ns=post))
        assert sum(r["send_to_ready_ns"] for r in replies) <= handler
        owners.append(dict(pid=pid, effects=sum(begin < j["record2"] <= end for j in own),
                           middle_handler_ns=handler, other_than_handler_ns=4*(end-begin)-handler,
                           reply_intervals=replies, published_before_pick_ns=area("pub_after", "pick_lower"),
                           workers_at_slowest_peer_cadence=handler / slowest,
                           five_worker_residence_excess_ns=max(0, handler - 5*slowest)))
    assert sum(o["effects"] for o in owners) == 512
    return dict(phase="instrumented warmup", middle_ns=end-begin, owners=owners,
                warm_river_middle_us={g: p["phases"]["middle_us"] for g, p in peers.items()},
                alignment_allowance_ns=slack)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path)
    parser.add_argument("decoder", type=Path)
    args = parser.parse_args()
    print(json.dumps(checked_worker_budget(args.evidence, args.decoder), indent=2))
