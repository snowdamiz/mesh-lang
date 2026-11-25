import unittest
from replay import overlap


class OverlapTest(unittest.TestCase):
    def test_partial_intervals_parallel_runners_and_other_lookup(self):
        events = [(10, "after", 0, 80), (10, "after", 0, 50), (10, "before", 20, 30),
                  (10, "lookup", 5, 25), (11, "after", 40, 90), (11, "after", 45, 60),
                  (11, "before", 48, 52)]
        self.assertEqual(overlap(events, 10, 70, [10, 11]), {
            "middle_us": 60, "either_runner_us": 38, "both_runners_us": 3,
            "combined_runner_us": 41, "combined_with_lookup_us": 10,
        })

    def test_touching_calls_dont_overlap(self):
        events = [(1, "after", 0, 10), (1, "after", 10, 20), (2, "after", 0, 20)]
        self.assertEqual(overlap(events, 0, 20, [1, 2])["either_runner_us"], 0)


if __name__ == "__main__":
    unittest.main()
