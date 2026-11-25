"""Check physical compression for the exact frozen ordinary slow fixture."""
from collections import Counter


def verify_codec(storage, engine, persisted, jobs, policy):
    storage.verify_snapshot(engine, persisted, jobs, policy)
    counts = Counter(row["compression"] for row in persisted["rows"])
    expected = {"lz4" if policy == "lz4" else None: 512}
    if counts != expected:
        raise ValueError(f"{engine}/{policy}: expected {expected}, observed {dict(counts)}")
    return {"uncompressed" if codec is None else codec: count for codec, count in counts.items()}


def write_landmarks(ledger, origin_us):
    first_writes = sorted(job["attempt_committed_us"] for job in ledger["jobs"])
    return {
        "commit_to_first_application_commit_ms": (first_writes[0] - origin_us) / 1000,
        "commit_to_64th_first_application_commit_ms": (first_writes[63] - origin_us) / 1000,
        "commit_to_final_first_application_commit_ms": (first_writes[-1] - origin_us) / 1000,
        "64th_to_final_first_application_commit_ms": (first_writes[-1] - first_writes[63]) / 1000,
    }
