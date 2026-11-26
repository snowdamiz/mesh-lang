"""Partition the split FILE pilot's enqueue delta and count jobs still awaiting a claim."""

import argparse
from hashlib import file_digest
import json
from pathlib import Path
import tarfile


RAW = "3b4a053cb2de7726f65a2fc8ea7f6da3e181b013a109896cb2edde64ce9b5775"
REPLAY = "580a771f9375b6ef6f9fc0b6716dbab9e28f2a07bcc4e3f9a3b1e5313f0b25ea"


def checked(path, digest):
    with path.open("rb") as stream:
        assert file_digest(stream, "sha256").hexdigest() == digest, path
    return path


def arm_cost(archive, name, score, cutoff):
    ledger = json.load(archive.extractfile(f"{name}/ledger.json"))
    stored = json.load(archive.extractfile(f"{name}/split-durable.json"))
    transaction, = ledger["transactions"]
    commit = transaction["committed_us"]
    enqueue = (commit - transaction["submitted_us"]) / 1000
    assert enqueue == score["enqueue_us"] / 1000
    statements = stored["statements"]
    elapsed = sum(row["total_exec_time"] for row in statements)
    writer, = [row for row in statements if row["query"].startswith("SELECT mesh_jobs.enqueue")]
    assert writer["calls"] == 64 and writer["total_exec_time"] == score["writer_executor_ms"]
    jobs = ledger["jobs"]
    assert len(jobs) == len({job["job_key"] for job in jobs}) == 64
    drain_us = max(job["settled_us"] for job in jobs) - commit
    assert drain_us == score["drain_us"]
    probe = score["availability_executor_ms"]
    return dict(enqueue=enqueue, writer=writer["total_exec_time"], probe=probe,
                other_sql=elapsed-writer["total_exec_time"]-probe, remaining=enqueue-elapsed,
                unstamped=sum(job["claimed_us"] > commit + cutoff for job in jobs),
                effects=sum(job["effect_committed_us"] <= commit + cutoff for job in jobs),
                late=(drain_us-cutoff)/1000)


def main(evidence):
    replay = json.loads(checked(evidence / "independent-replay.json", REPLAY).read_bytes())
    assert replay["complete"] and replay["raw_sha256"] == RAW and len(replay["arms"]) == 18
    deadlines = json.loads(Path(__file__).with_name("ordinary-cursor-deadlines.json").read_bytes())
    cutoff = max(r["budget_from_commit_us"] for r in deadlines
                 if (r["policy"], r["peer"], r["target"]) == ("EXTERNAL", "oban", "1.20x"))
    print(f"Historical Oban drain cutoff {cutoff} us")
    print("block control ON-minus-OFF ms: enqueue writer probe other_sql remaining | ON unstamped effects drain_late_ms")
    with tarfile.open(checked(evidence / "raw.tar.gz", RAW)) as archive:
        blocks = {}
        for name, score in replay["arms"].items():
            group, _, lane = name.split("-", 2)
            blocks.setdefault(int(group[1:]), {})[lane] = arm_cost(archive, name, score, cutoff)
    for group, arms in sorted(blocks.items()):
        control = min(("off-a", "off-b"), key=lambda lane: arms[lane]["enqueue"])
        on = arms["on"]
        delta = [on[key] - arms[control][key] for key in ("enqueue", "writer", "probe", "other_sql", "remaining")]
        assert abs(delta[0] - sum(delta[1:])) < 1e-7
        print(group, control, *(f"{ms:.3f}" for ms in delta), "|", on["unstamped"], on["effects"], f"{on['late']:.3f}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path)
    main(parser.parse_args().evidence)
