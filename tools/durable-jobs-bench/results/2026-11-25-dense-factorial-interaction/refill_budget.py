"""Bound the producer target and match subsequent claim stamps to terminal commits."""
from bisect import bisect_left
from collections import Counter, defaultdict
import json
import tarfile

from peer_writes import TRIAL, replay


def paired_widths(jobs, horizon_us):
    starts = Counter(j["claimed_us"] for j in jobs)
    ends = Counter((j["settled_us"], j["settled_xid"]) for j in jobs)
    unused = sorted(starts)
    pairs = seats = 0
    for (end, _), freed in sorted(ends.items()):
        i = bisect_left(unused, end)
        if i < len(unused) and unused[i] - end <= horizon_us:
            stamp = unused.pop(i)
            pairs += 1
            seats += min(starts[stamp], freed)
    return dict(pairs=pairs, seats=seats)


def budgets():
    data = []
    for block in replay()["blocks"]:
        group = block["group"]
        mesh, river = (block["arms"][k] for k in ("combined", "river-2x32"))
        archive, = (TRIAL / "blocks").glob(f"{group:02}-noop.*/measured-combined/measured-mesh/raw.tar.gz")
        with tarfile.open(archive) as raw:
            jobs = json.load(raw.extractfile("adapter/ledger.json"))["jobs"]
        workers = defaultdict(list)
        for job in jobs:
            workers[job["worker_pid"]].append(job)
        assert len(workers) == 2
        windows = {}
        for ms in (1, 3, 8):
            tally = Counter()
            for members in workers.values():
                tally.update(paired_widths(members, ms * 1000))
            windows[str(ms)] = dict(tally)
        data.append(dict(group=group, stamp_matches=windows,
                         saving_for_20pct_submit_ms=round(mesh["submit"]-river["submit"]/1.2, 3),
                         drain_saving_if_producers_equal_ms=round(mesh["drain"]+river["enqueue"]-river["submit"]/1.2, 3)))
    return data


if __name__ == "__main__":
    print(json.dumps(budgets(), indent=2, sort_keys=True))
