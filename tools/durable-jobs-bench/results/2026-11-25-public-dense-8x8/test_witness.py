"""Exercise the roster witness with the released canonical PostgreSQL ledgers."""
from copy import deepcopy
import tarfile
import unittest

from controller import ENGINES, HERE, check, run, score
from witness import BULK, SCALAR, logger_off

ARCHIVES = (HERE.parent / "2026-11-25-public-dense-stock-logical-key/continuation/evidence/native/blocks")


def load(path):
    with tarfile.open(path) as raw:
        def get(name):
            return run.file_json(raw, "adapter/" + name + ".json")

        jobs = get("ledger")["jobs"]
        queue = get("queue-policy")
        pids = [item["pid"] for item in get("process-identity")[1:]]
        logs = [raw.extractfile(f"adapter/runner-{i}.log").read().decode() for i in range(8)]
        namespace = get("raw")["namespace"]
    return jobs, queue, logs, pids, namespace


class CohortWitness(unittest.TestCase):
    def test_all_six_actual_individual_arms(self):
        paths = sorted(ARCHIVES.glob("*/*/measured-mesh/raw.tar.gz"))
        self.assertEqual(len(paths), 6)
        for path in paths:
            with self.subTest(path=path):
                jobs, queue, logs, pids, namespace = load(path)
                result = logger_off(jobs, queue, logs, pids, namespace, False)
                self.assertEqual(len(result["runners"]), 8)
                self.assertEqual(result["widths"], {1: 1024})
                self.assertEqual(result["scalar_singletons"], 1024)

    def test_rejects_cross_runner_and_forged_success(self):
        jobs, queue, logs, pids, namespace = load(next(ARCHIVES.glob("*/*/measured-mesh/raw.tar.gz")))
        mutated = deepcopy(jobs)
        other = next(job for job in mutated if job["worker_pid"] != mutated[0]["worker_pid"])
        other["settled_xid"] = mutated[0]["settled_xid"]
        with self.assertRaisesRegex(ValueError, "mixed runners"):
            logger_off(mutated, queue, logs, pids, namespace, False)
        with self.assertRaisesRegex(ValueError, "logger-on"):
            logger_off(jobs, queue, [logs[0] + "jobs_grouped_writer {}\n", *logs[1:]],
                       pids, namespace, False)
        altered = deepcopy(queue)
        scalar = next(stmt for stmt in altered["queue_statements"] if stmt["query"] == SCALAR)
        scalar["rows"] -= 1
        with self.assertRaisesRegex(ValueError, "SQL and committed XIDs"):
            logger_off(jobs, altered, logs, pids, namespace, False)

    def test_width_one_and_grouped_sql_reconcile(self):
        jobs, queue, logs, pids, namespace = load(next(ARCHIVES.glob("*/*/measured-mesh/raw.tar.gz")))
        first, second = sorted((job for job in jobs if job["worker_pid"] == pids[0]),
                               key=lambda job: job["settled_us"])[-2:]
        first["settled_xid"], first["settled_us"] = second["settled_xid"], second["settled_us"]
        scalar = next(stmt for stmt in queue["queue_statements"] if stmt["query"] == SCALAR)
        scalar.update(calls=1021, rows=1021)
        queue["queue_statements"].append(dict(query=BULK, calls=2, rows=3))
        queue.update(largest_terminal_cohort=2, terminal_xids=1023, completion_cohort_queries=2)
        result = logger_off(jobs, queue, logs, pids, namespace, True)
        self.assertEqual(result["widths"], {1: 1022, 2: 1})
        self.assertEqual(result["bulk_singletons"], 1)
        self.assertEqual(result["scalar_singletons"], 1021)

    def test_actual_stock_peers_replay_under_the_same_inventory(self):
        base = HERE.parent / "2026-11-23-dense-matched-topology/evidence/stock/blocks"
        for peer, engine in ENGINES.items():
            paths = sorted(base.glob(f"*/measured-{peer}-8x8/measured-{engine}"))
            self.assertEqual(len(paths), 5)
            for path in paths:
                row = score(path, "measured", peer, "peers", None)
                self.assertEqual(row["completed"], 1024)
                self.assertGreater(row["submit_us"], row["drain_us"])

    def test_exact_setup_and_stock_inventories(self):
        build, routes = check()
        self.assertEqual(len(routes), 2)
        self.assertTrue(build["files"]["mesh"].startswith("4924b588"))


if __name__ == "__main__":
    unittest.main()
