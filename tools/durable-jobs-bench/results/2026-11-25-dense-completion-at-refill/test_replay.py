import unittest

from replay import merge_timeline


def jobs(claim, ready):
    return [dict(claimed_us=claim, settled_us=t) for t in ready]


class TimelineTests(unittest.TestCase):
    def test_claim_needs_a_charged_slot(self):
        timeline = jobs(0, [15] + list(range(30, 61))) + jobs(20, [70])
        fixed = merge_timeline(timeline, 8, False)
        refill = merge_timeline(timeline, 8, True)
        self.assertEqual(fixed["seats_unavailable_at_original_grants"], 1)
        self.assertEqual(refill["seats_unavailable_at_original_grants"], 0)

    def test_eight_simultaneous_receipts_fill_one_group(self):
        self.assertEqual(merge_timeline(jobs(0, [50] * 8), 8000, True)["groups"], 1)

    def test_deadline_precedes_later_ready(self):
        self.assertEqual(merge_timeline(jobs(0, [10, 19]), 8, False)["groups"], 2)


if __name__ == "__main__":
    unittest.main()
