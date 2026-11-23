"""Compare the replay against small exact partitions and event-by-event occupancy."""

import itertools
import random
from functools import lru_cache

from replay import minimum_turns, residence


def exact_turns(windows, width):
    @lru_cache(None)
    def partition(indices):
        if not indices:
            return 0
        best = len(indices)
        # Enumerate all valid cohorts containing the first outstanding job.
        for size in range(min(width, len(indices))):
            for others in itertools.combinations(indices[1:], size):
                group = (indices[0], *others)
                if max(windows[i][0] for i in group) > min(windows[i][1] for i in group):
                    continue
                best = min(best, 1 + partition(tuple(i for i in indices if i not in group)))
        return best

    return partition(tuple(range(len(windows))))


def main():
    intervals = [(s, e) for s in range(3) for e in range(s, 3)]
    cases = 0
    for size in range(1, 6):
        for windows in itertools.combinations_with_replacement(intervals, size):
            for width in range(1, 4):
                assert minimum_turns(windows, width) == exact_turns(windows, width)
                cases += 1
    rng = random.Random(17)
    for _ in range(200):
        windows = sorted(rng.choices(intervals, k=rng.randrange(6, 10)))
        width = rng.randrange(1, 6)
        assert minimum_turns(windows, width) == exact_turns(windows, width)
        cases += 1

    jobs = [
        {"worker_pid": 1, "claimed_us": 1, "entered_us": 2, "settled_us": 4},
        {"worker_pid": 1, "claimed_us": 1, "entered_us": 3, "settled_us": 5},
        {"worker_pid": 2, "claimed_us": 3, "entered_us": 4, "settled_us": 7},
    ]
    for start, end, delay in itertools.product((0, 2, 4), (5, 7, 10), (0, 1, 999)):
        active = [[j["worker_pid"] for j in jobs
                   if min(j["claimed_us"] + delay, j["entered_us"]) <= t < j["settled_us"]]
                  for t in range(start, end)]
        observed = residence(jobs, start, end, delay)
        assert observed == {"occupied_job_us": sum(map(len, active)),
                            "empty_runner_us": sum(2 - len(set(group)) for group in active),
                            "peak_resident_jobs": max(map(len, active))}
    print(f"Verified {cases} interval partitions and 27 residence integrations.")


if __name__ == "__main__":
    main()
