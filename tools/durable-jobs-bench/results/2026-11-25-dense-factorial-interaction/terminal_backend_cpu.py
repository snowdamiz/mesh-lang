"""Join issued Jobs statements to the sampled server processes by live connection."""
from bisect import bisect_right
from collections import Counter
from hashlib import sha256
from io import BytesIO
import json
import subprocess
import tarfile

ROOT = "c14aabdb5:tools/durable-jobs-bench/results/2026-11-25-dense-cross-runner-disposition/released/"
TERMINAL = {"scalar", "local_group", "crossed", "cross_commit", "xid_status"}


def summarize(data):
    ranks = sorted(j["effect_us"] for j in data["ledger"])
    assert len(ranks) == 1024
    begin, end = ranks[255] * 1000, ranks[767] * 1000
    frames = data["frames"]
    assert [f["rank"] for f in frames] == [256, 384, 512, 640, 768]
    assert all(a["end_ns"] < b["begin_ns"] and a["epoch_ns"] < b["epoch_ns"]
               for a, b in zip(frames, frames[1:]))
    # epoch_ns follows the last read in its frame. Require the previous frame
    # to have finished after the lower endpoint and this one before the upper.
    interior = [b for a, b in zip(frames, frames[1:])
                if begin <= a["epoch_ns"] and b["epoch_ns"] <= end]
    assert len(interior) >= 2
    terminals = {(r["pid"], r["connection"]) for r in data["jobs_sql"] if r["kind"] in TERMINAL}
    ready = {(r["pid"], r["connection"]): r for r in data["ready"] if r["role"] == "parity_subject"}
    assert len(ready) == 32 and terminals <= ready.keys()
    used = {str(ready[key]["backend"]) for key in terminals}

    roster = {str(r["backend"]): r["role"] for r in data["roster"]["sessions"]}
    assert len(used) == 8 and len(set(r["backend"] for r in ready.values())) == len(ready)
    assert all(roster[str(r["backend"])] == r["role"] for r in data["ready"])

    def census(a, b):
        costs, counts = Counter(), Counter()
        assert a["postgres"].keys() == b["postgres"].keys() and len(a["postgres"]) == 65
        for pid, first in a["postgres"].items():
            last = b["postgres"][pid]
            assert first["start"] == last["start"] and first["role"] == last["role"] == roster[pid]
            role = first["role"]
            if role == "parity_subject":
                role = "jobs_terminal_connection" if pid in used else "jobs_other_connection"
            ns = last["running_ns"] - first["running_ns"]
            assert ns >= 0
            costs[role] += ns
            counts[role] += 1
        return dict(first_frame_rank=a["rank"], last_frame_rank=b["rank"],
                    durable_effects_at_frame_ends=[bisect_right(ranks, f["epoch_ns"]//1000) for f in (a, b)],
                    frame_end_spacing_ms=(b["epoch_ns"]-a["epoch_ns"])/1e6,
                    backend_count=dict(counts), backend_scheduled_ms={k:v/1e6 for k,v in costs.items()})
    return dict(effect_span_ms=(end-begin)/1e6, outer_frames=census(frames[0], frames[-1]),
                entirely_inside_middle=census(interior[0], interior[-1]))


def main():
    def blob(name):
        return subprocess.check_output(["git", "show", ROOT+name])
    release = json.loads(blob("controller-release.json"))
    raw = blob("raw.tar.gz")
    assert release["exit_status"] == 1 and sha256(raw).hexdigest() == release["raw_sha256"]
    with tarfile.open(fileobj=BytesIO(raw)) as archive:
        result = {lane: summarize(json.load(archive.extractfile(
            f"./semantic-root/cross/active/{lane}/telemetry.json")))
            for lane in ("original", "off", "observe", "on")}
    print(json.dumps(dict(raw_sha256=release["raw_sha256"], lanes=result), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
