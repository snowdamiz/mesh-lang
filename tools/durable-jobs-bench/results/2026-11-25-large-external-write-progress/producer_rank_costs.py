"""Compare equal-width intervals between stored LARGE FILE enqueue timestamps."""

import hashlib
import io
import json
import subprocess
import tarfile
from statistics import median


COMMIT = "d09f644cba38daf1b48b55ec05d8ee99bcd25400"
SOURCE = "tools/durable-jobs-bench/results/2026-11-25-large-file-producer-budget/release-8fb38211f99d"
LANES = ("original-off", "original-on", "off-a", "off-b", "on-a", "on-b")


def blob(name):
    return subprocess.check_output(["git", "show", f"{COMMIT}:{SOURCE}/{name}"])


def digest(data):
    return hashlib.sha256(data).hexdigest()


def spread(values):
    return [min(values), median(values), max(values)]


def arm_data(archive, arm):
    def read(name):
        return json.load(archive.extractfile(f"./{arm}/{name}.json"))

    ledger = read("ledger")
    stored = read("stored-enqueue-stamps")
    txn, = ledger["transactions"]
    rows = stored["rows"]
    jobs = {job["job_key"]: job for job in ledger["jobs"]}
    assert len(txn["enqueued_ids"]) == len(jobs) == len(rows) == 64
    assert len(set(txn["enqueued_ids"].values())) == 64
    for rank, row in enumerate(rows, 1):
        key = f"job_{rank - 1:06}"
        job = jobs[key]
        assert row["position"] == rank and job["id"] == row["id"] == txn["enqueued_ids"][key]
        assert row["attempt"] == job["attempts"] == job["effects"] == 1
        assert row["state"] == job["state"] == "completed"
        assert row["inserted_at_ms"] == row["scheduled_at_ms"]
        assert job["settled_us"] * 1000 <= stored["collected"][1]
    stamps = [row["inserted_at_ms"] for row in rows]
    gaps = [b - a for a, b in zip(stamps, stamps[1:])]
    assert min(gaps) >= 0
    submitted, committed = txn["submitted_us"], txn["committed_us"]
    assert submitted <= stamps[0] * 1000 + 999 and stamps[-1] * 1000 <= committed
    return dict(gaps=gaps, enqueue_us=committed - submitted,
                prefix_us=stamps[0] * 1000 - submitted, tail_us=committed - stamps[-1] * 1000)


def compare(arms):
    by_lane = {}
    for lane in LANES:
        subset = [arm for name, arm in arms.items() if name.split("-", 2)[2] == lane]
        assert len(subset) == 6
        gaps = [arm["gaps"] for arm in subset]
        by_lane[lane] = dict(
            first_five_minus_next_five_ms=spread([sum(g[:5]) - sum(g[5:10]) for g in gaps]),
            first_fifteen_minus_last_fifteen_ms=spread([sum(g[:15]) - sum(g[-15:]) for g in gaps]),
            median_gap_by_rank_band_ms=[median([median(g[lo:hi]) for g in gaps])
                                        for lo, hi in [(0, 5), (5, 10), (10, 15), (15, 31), (31, 47), (47, 63)]],
        )

    pairs = []
    for group in range(1, 7):
        def off(side):
            arm, = [row for name, row in arms.items()
                    if name.startswith(f"g{group}-") and name.endswith(f"-off-{side}")]
            return arm

        a, b = off("a"), off("b")
        delta = [x - y for x, y in zip(a["gaps"], b["gaps"])]
        prefix_us = a["prefix_us"] - b["prefix_us"]
        tail_us = a["tail_us"] - b["tail_us"]
        enqueue_us = a["enqueue_us"] - b["enqueue_us"]
        assert prefix_us + 1000 * sum(delta) + tail_us == enqueue_us
        pairs.append(dict(group=group, a_minus_b_enqueue_us=enqueue_us, prefix_us=prefix_us,
                          recurring_ms=sum(delta), tail_us=tail_us,
                          a_slower_by_at_least_2ms_intervals=sum(value >= 2 for value in delta),
                          b_slower_by_at_least_2ms_intervals=sum(value <= -2 for value in delta),
                          recurring_by_rank_ms=[sum(delta[lo:hi]) for lo, hi in
                                                [(0, 7), (7, 15), (15, 31), (31, 47), (47, 63)]]))
    return by_lane, pairs


def main():
    release = blob("controller-release.json")
    adjudication = json.loads(blob("adjudication.json"))
    status = json.loads(blob("native-status.json"))
    raw = blob("raw.tar.gz")
    assert digest(release) == adjudication["release_sha256"]
    assert json.loads(release)["exit_status"] == 1
    assert status["exit_status"] == status["export_exit_status"] == 0
    assert digest(raw) == status["raw_sha256"] == adjudication["raw_sha256"]
    names = sorted(adjudication["recurring"]["by_arm"])
    assert len(names) == adjudication["corrected_raw_accepted"] == 36
    with tarfile.open(fileobj=io.BytesIO(raw), mode="r:gz") as archive:
        arms = {name: arm_data(archive, name) for name in names}
    lanes, pairs = compare(arms)
    early = {name: [sum(a["gaps"][:5]) - sum(a["gaps"][5:10]),
                    sum(a["gaps"][:15]) - sum(a["gaps"][-15:])] for name, a in arms.items()}
    print(json.dumps(dict(source=COMMIT, raw_sha256=digest(raw), release_sha256=digest(release),
                          lanes=lanes, off_duplicate_pairs=pairs, early_differences_ms=early),
                     indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
