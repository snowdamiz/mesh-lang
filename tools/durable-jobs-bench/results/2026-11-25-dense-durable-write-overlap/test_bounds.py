from collections import Counter
from functools import cache
from itertools import combinations
import random
import unittest

from replay import minimum_events


def exact_minimum(windows, cap):
    # Exhaustively partition writes into cohorts that have a common time and fit each runner.
    @cache
    def remaining(indices):
        if not indices:
            return 0
        smallest = len(indices)
        for size in range(1, len(indices) + 1):
            for rest in combinations(indices[1:], size - 1):
                selected = {indices[0], *rest}
                group = [windows[i] for i in selected]
                if max(x[0] for x in group) > min(x[1] for x in group):
                    continue
                if cap is not None and max(Counter(x[2] for x in group).values()) > cap:
                    continue
                smallest = min(smallest, 1 + remaining(tuple(i for i in indices if i not in selected)))
        return smallest
    return remaining(tuple(range(len(windows))))


class BoundsTest(unittest.TestCase):
    def test_limits_apply_separately_to_each_runner(self):
        self.assertEqual(minimum_events([(0, 1, pid) for pid in (1, 1, 2, 2)], cap=2), 1)
        self.assertEqual(minimum_events([(0, 1, 1)] * 3 + [(2, 3, 2)] * 3, cap=2), 4)

    def test_rounding_outward_cannot_increase_minimum(self):
        self.assertEqual(minimum_events([(0, 1, 1), (1, 2, 2)]), 1)
        self.assertEqual(minimum_events([(0, 1, 1), (2, 3, 2)]), 2)
        with self.assertRaises(ValueError):
            minimum_events([(5, 4, 1)])

    def test_against_exhaustive_small_partitions(self):
        rng = random.Random(2719)
        for i in range(75):
            spans = [(*sorted(rng.sample(range(10), 2)), rng.randrange(2)) for _ in range(1 + i % 8)]
            for cap in (1, 2, None):
                self.assertEqual(minimum_events(spans, cap), exact_minimum(spans, cap), (spans, cap))


if __name__ == "__main__":
    unittest.main()
