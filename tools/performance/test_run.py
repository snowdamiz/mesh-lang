import argparse
from collections import Counter
import math
import unittest

import run


class BenchmarkContractTests(unittest.TestCase):
    def test_actor_checks_counter_and_sampling_boundary(self):
        trial = run.Trial("mesh", "actor", 1, 1, pairs=3, iterations=129)
        result = {"ops": 387, "checksum": 25155, "elapsed_ns": 1, "rtt_ns": [1] * 9}
        run.validate_result(trial, [], result)
        with self.assertRaisesRegex(ValueError, "9 nonnegative integer RTT samples"):
            run.validate_result(trial, [], {**result, "rtt_ns": [1] * 6})
        with self.assertRaisesRegex(ValueError, "checksum=25155"):
            run.validate_result(trial, [], {**result, "checksum": 387})

    def test_cpu_oracle_wraps_and_checks_recovery_evidence(self):
        fixtures = run.fixture_records()
        self.assertEqual(len({record["id"] for record in fixtures}), 4)
        self.assertTrue(all(1000 <= len(run.compact(record)) <= 1048 for record in fixtures))
        run.validate_result(
            run.Trial("rust", "cpu", 4, 1, iterations=13),
            fixtures, {"ops": 13, "checksum": 223522318, "elapsed_ns": 1},
        )
        with self.assertRaisesRegex(ValueError, "checksum=1"):
            run.validate_result(
                run.Trial("go", "recovery", 4, 1),
                fixtures, {"ops": 1, "checksum": 0, "elapsed_ns": 1},
            )

    def test_shuffled_blocks_include_every_configuration_once(self):
        args = argparse.Namespace(
            command="run", seed=51, blocks=5, pairs=64, actor_iters=129, cpu_iters=13,
            http_rates=[1, 100], sustained_rate=10,
        )
        trials = run.trial_order(args)
        counts = Counter((t.language, t.mode, t.cpus, t.pairs, t.rate) for t in trials)
        self.assertEqual(len(counts), 4 * (2 * (3 + 2) + 2))
        self.assertEqual(set(counts.values()), {1, 5})
        self.assertEqual({t.pairs for t in trials if t.mode == "actor"}, {1, 64})
        blocks = [[t.language for t in trials if t.block == n] for n in range(1, 6)]
        self.assertEqual(len({tuple(block) for block in blocks}), 5)
        self.assertEqual({t.cpus for t in trials if t.mode == "sustained"}, {4})
        self.assertEqual(sum(t.mode == "sustained" for t in trials), 4)

    def test_goodput_and_slo_count_all_failed_load(self):
        metrics = {
            name: {"values": {"count": count}}
            for name, count in {
                "score_started": 9996, "score_success": 9994, "score_errors": 1,
                "score_within_slo": 9980, "iterations": 9995, "dropped_iterations": 3,
            }.items()
        }
        metrics["score_e2e_ms"] = {"values": {"p(99)": 40}}
        result = run.load_result({"metrics": metrics}, 10_000, 10)
        self.assertEqual(result["offered"], 9999)
        self.assertEqual(result["interrupted"], 1)
        self.assertEqual(result["missing"], 1)
        self.assertTrue(math.isclose(result["error_fraction"], 0.0006))
        self.assertEqual(result["slo_goodput_per_s"], 998)
        self.assertTrue(result["slo_met"])
        metrics["score_success"]["values"]["count"] = 9990
        self.assertFalse(run.load_result({"metrics": metrics}, 10_000, 10)["slo_met"])
        metrics["score_success"]["values"]["count"] = 9994
        metrics["score_e2e_ms"]["values"]["p(99)"] = 51
        self.assertFalse(run.load_result({"metrics": metrics}, 10_000, 10)["slo_met"])


if __name__ == "__main__":
    unittest.main()
