import random

def inputs(workload, slots, seed, smoke=False):
    count = max(workload["minimum_jobs"], slots * workload["jobs_per_slot"])
    if smoke:
        count = 4
    rng = random.Random(seed)
    batch_size = 1 if workload["mode"] == "sparse" else count
    batches = []
    for start in range(0, count, batch_size):
        jobs = [{
            "key": f"job_{index:06d}",
            "payload": rng.randbytes((workload["payload_bytes"] + 1) // 2).hex()[:workload["payload_bytes"]],
            "hold_ms": workload["hold_ms"],
            "fail_first": False,
        } for index in range(start, min(start + batch_size, count))]
        batches.append({"request_id": f"batch_{start:06d}", "jobs": jobs, "rollback": False})
    return batches

