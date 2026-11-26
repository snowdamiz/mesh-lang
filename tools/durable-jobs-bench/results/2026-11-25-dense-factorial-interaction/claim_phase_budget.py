"""Price productive claim phases in the released two-owner observer captures."""
import hashlib
import json
import math
import statistics
import sys
import tarfile
from collections import defaultdict
from pathlib import Path

RELEASE = "ad8c5b32cc88344ca01d4d47cedc24c3bc804be00e2d6cd3f7a8123ab18206a2"
VERIFIED = "8d8ebd9da5d3a1883d8f45a66ce63a5681d3e49021825445fa3c3e0845196013"
RAW = "b99a9f68a6472b3a49324dbe769e7d3a3755a315c64b02da8bd3f6dbaf1da29f"


def checked(data, digest):
    if hashlib.sha256(data).hexdigest() != digest:
        raise ValueError(f"input differs from {digest}")
    return data


def distribution(values):
    ordered = sorted(values)
    return {
        "sum_ms": round(sum(values) / 1e6, 3),
        "median_ms": round(statistics.median(values) / 1e6, 3),
        "p95_ms": round(ordered[math.ceil(.95 * len(values)) - 1] / 1e6, 3),
    }


def analyze(arm, jobs):
    by_grant = defaultdict(list)
    for job in jobs:
        by_grant[int(job["worker_pid"]), int(job["claimed_us"])].append(job)
    assert len(jobs) == 1024
    assert len({j[k] for j in jobs for k in ("attempt_xid", "effect_xid")}) == 2048
    start, end = arm["write_edges_us"]
    effects = sorted(j["effect_committed_us"] for j in jobs)
    assert [start, end] == [effects[255], effects[767]]
    low, high = arm["clock_offsets_ns"]["utc"]
    phases = defaultdict(lambda: defaultdict(list))
    client_holds = []
    for t in arm["turns"]:
        if not t["rows"]:
            continue
        cohort = by_grant.pop((t["pid"], t["grant_stamp_us"]))
        assert len(cohort) == t["rows"]
        if t["reserved_ns"] + low < start * 1000 or t["received_ns"] + high > end * 1000:
            continue
        partial = t["prelock_receipt"] is not None
        opening = "prelock" if partial else "begin"
        path = phases[opening]
        names = ["reserved", opening + "_send", opening + "_receipt", "select_send",
                 "select_receipt", "commit_send", "commit_receipt", "dispatcher_reply"]
        marks = [e for e in t["events"] if e["mark"] in names]
        assert [e["mark"] for e in marks] == names
        stamps = [e["ns"] for e in marks]
        assert stamps == sorted(stamps)
        for i, (a, b) in enumerate(zip(names, names[1:])):
            path[a + "_to_" + b].append(stamps[i + 1] - stamps[i])
        client_holds.append((stamps[4], stamps[5]))
        if partial:
            client_holds.append((stamps[2], stamps[3]))
        first_entry = 1000 * min(j["entered_us"] for j in cohort)
        for i in (6, 7):
            for label, offset in (("lower", high), ("upper", low)):
                path[names[i] + "_to_first_entry_" + label].append(
                    max(0, first_entry - stamps[i] - offset))
    assert not by_grant
    client_holds.sort()
    assert all(a[1] <= b[0] for a, b in zip(client_holds, client_holds[1:]))
    return {
        "middle_ms": (end - start) / 1000,
        "client_lock_held_between_exchanges_ms": round(sum(b-a for a,b in client_holds) / 1e6, 3),
        "paths": {name: {"count": len(next(iter(data.values()))),
                         "intervals": {k: distribution(v) for k,v in data.items()}}
                  for name, data in phases.items()},
    }


def main(evidence):
    release = json.loads(checked((evidence / "controller-release.json").read_bytes(), RELEASE))
    verified = json.loads(checked((evidence / "verified.json").read_bytes(), VERIFIED))
    assert release["exit_status"] == 0 and verified["raw_sha256"] == RAW
    raw = evidence / "raw.tar.gz"
    with raw.open("rb") as stream:
        assert hashlib.file_digest(stream, "sha256").hexdigest() == RAW
    groups = []
    with tarfile.open(raw) as tar:
        for i, group in enumerate(verified["result"]["groups"], 1):
            arm = group["arms"]["observe/measured"]
            member = tar.extractfile(f"./g{i}-observe-measured/ledger.json")
            ledger = json.loads(checked(member.read(), arm["hashes"]["ledger.json"]))
            groups.append(analyze(arm, ledger["jobs"]))
    print(json.dumps({"raw_sha256": RAW, "verdict": verified["verdict"], "groups": groups},
                     indent=2, sort_keys=True))


if __name__ == "__main__":
    main(Path(sys.argv[1]))
