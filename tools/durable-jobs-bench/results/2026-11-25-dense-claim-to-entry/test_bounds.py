import unittest

from replay import certify_batches, upper_bounds


def job(pid, claim, enter, settle):
    return dict(worker_pid=pid, claimed_us=claim, entered_us=enter, settled_us=settle,
                state="completed", attempts=1)


class GrantBounds(unittest.TestCase):
    def test_rows_do_not_certify_a_timestamp_collision(self):
        rows = [job(10, 1000, 6000, 9000), job(10, 1000, 6200, 9200)]
        self.assertEqual(len(certify_batches(rows, calls=3, rows=4)), 1)
        with self.assertRaises(AssertionError):
            certify_batches(rows, calls=4, rows=4)

    def test_other_runner_only_bounds_database_commit(self):
        jobs = [job(1, 1000, 6600, 8000), job(1, 1000, 9800, 11000),
                job(2, 3000, 4200, 9000)]
        buckets = certify_batches(jobs, calls=2, rows=3)
        pid, stamp, batch, entry, durable, consumed, spawned = next(upper_bounds(buckets))
        self.assertEqual((pid, stamp, len(batch), entry), (1, 1000, 2, 6600))
        self.assertEqual((durable, consumed, spawned), (4000, 6600, 8000))

    def test_same_millisecond_on_second_runner_is_not_a_later_grant(self):
        a = job(1, 1000, 5000, 6800)
        b = job(2, 1000, 2200, 4600)
        batch = next(upper_bounds(certify_batches([a, b], calls=2, rows=2)))
        self.assertEqual(batch[-3:], (5000, 5000, 6800))

    def test_next_local_claim_proves_prior_spawns_even_if_no_actor_has_entered(self):
        jobs = [job(1, 1000, 7500, 9500), job(1, 3000, 6200, 6800)]
        batch = next(upper_bounds(certify_batches(jobs, calls=2, rows=2)))
        self.assertEqual(batch[-3:], (4000, 4000, 4000))


if __name__ == "__main__":
    unittest.main()
