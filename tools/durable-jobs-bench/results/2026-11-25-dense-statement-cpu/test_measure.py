"""Synthetic controls for the exit report and perturbation rejection paths."""
import json
import unittest

from cpu_adapter import label
from measure import perturbation, read_report


class Controls(unittest.TestCase):
    def test_cpu_conservation_and_loss(self):
        header = dict(version=1, enabled=1, postmaster=90, registered=1, parallel_seen=0,
                      children_cpu_ns=10_000, postmaster_cpu_ns=500)
        session = dict(session=0, pid=91, login="cpu", database="mesh", cpu_ns=10_000,
                       nested=8, auth=0, exit=0, exited=1, clock_failed=0)
        stmt = dict(slot=0, role=10, db=12, query="-5", cpu_ns=8000, calls=[1, 1, 0, 1, 0], errors=1)

        def parse(*rows):
            return read_report("\n".join(map(json.dumps, rows)), True)

        self.assertEqual(parse(header, session, stmt)[2], [stmt])
        for rows in ((header, session | {"exited": 0}, stmt),
                     (header, session, stmt | {"cpu_ns": 10_001}),
                     (header, session, stmt | {"role": 0}),
                     (header, session, stmt, stmt),
                     (header | {"registered": 2}, session, stmt)):
            with self.assertRaises(ValueError):
                parse(*rows)

    def test_control_spread_and_both_shift_directions(self):
        self.assertTrue(perturbation(100, 104, 102)["passes"])
        self.assertFalse(perturbation(100, 94, 102)["passes"])
        self.assertFalse(perturbation(100, 107, 102)["passes"])
        self.assertFalse(perturbation(90, 100, 110)["passes"])

    def test_shape_roles(self):
        for database, statement, expected in [
            ("mesh", "SELECT coalesce(mesh_jobs.settle_state($1), 'stale')", "settle"),
            ("mesh", "SELECT * FROM mesh_jobs.claim_batch_set_v1($1)", "grant"),
            ("mesh", "INSERT INTO public.mesh_jobs_parity_attempts VALUES ($1)", "application-write"),
            ("river", "WITH picked AS (SELECT id FROM job.river_job FOR UPDATE SKIP LOCKED) "
                      "UPDATE job.river_job SET attempted_at = $1", "grant"),
            ("river", "UPDATE job.river_job SET finalized_at = CASE WHEN job_input.finalized_at_do_update "
                      "THEN job_input.finalized_at ELSE river_job.finalized_at END", "settle"),
            ("river", "UPDATE job.river_job SET finalized_at = updated_job.finalized_at", "other"),
            ("river", "SELECT count(*) FROM job.river_job WHERE finalized_at IS NOT NULL", "other"),
        ]:
            self.assertEqual(label(database, statement), expected)


if __name__ == "__main__":
    unittest.main()
