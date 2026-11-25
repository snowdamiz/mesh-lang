import unittest

from xid_bounds import brackets


class AssignmentBounds(unittest.TestCase):
    def test_overlapping_transactions_with_reversed_commit_order(self):
        rows = [(20, "first", 0, 90), (18, "second", 30, 75), (22, "first", 5, 41)]
        self.assertEqual([(r[0], r[4], r[5]) for r in brackets(rows)],
                         [(18, 30, 41), (20, 30, 41), (22, 30, 41)])

    def test_duplicate_xids(self):
        with self.assertRaises(AssertionError):
            list(brackets([(9, "first", 1, 3), (9, "second", 4, 6)]))

    def test_contradictory_clock_witnesses(self):
        with self.assertRaises(AssertionError):
            list(brackets([(7, "first", 9, 12), (8, "second", 3, 7)]))

    def test_wrapped_range(self):
        with self.assertRaises(AssertionError):
            list(brackets([(3, "second", 2, 8), (2**32 - 1, "first", 1, 2)]))


if __name__ == "__main__":
    unittest.main()
