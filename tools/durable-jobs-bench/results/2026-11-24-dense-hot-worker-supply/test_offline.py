"""Verify the gate, stage contract, and existing reconciler without a workload."""
import importlib.util
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

import controller as pilot


class OfflineTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.series = pilot.run.read_json(pilot.retry.RESULTS / "series.json")
        cls.config = pilot.config_for(cls.series["config"])
        cls.gate = pilot.run.read_json(pilot.PREREQUISITE)
        cls.attempt = pilot.retry.RESULTS / "blocks" / cls.gate["blocks"]["6"]["attempt"]

    def test_only_runner_child_bounds_vary(self):
        path = pilot.HERE / "stage-arm.py"
        spec = importlib.util.spec_from_file_location("runner_stage", path)
        stage = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(stage)
        base_env = dict.fromkeys(stage.KEYS, "2") | {"MESH_EFFECT_LANES": "16", "JOBS_PARITY_SLOTS": "32"}
        self.assertEqual(stage.runner_env(base_env, "2"), base_env)
        self.assertEqual(stage.runner_env(base_env, "4"), base_env | dict.fromkeys(stage.KEYS, "4"))
        self.assertEqual({base_env[key] for key in stage.KEYS}, {"2"})
        with self.assertRaisesRegex(ValueError, "scheduler bounds"):
            stage.runner_env(base_env, "8")
        with self.assertRaisesRegex(ValueError, "scheduler bounds"):
            stage.runner_env({}, "4")

    def test_balanced_plan_and_strict_gain_gate(self):
        schedule = pilot.schedule()
        for a, b in zip(schedule[::2], schedule[1::2], strict=True):
            self.assertEqual(a["order"], b["order"][::-1])
        for label in pilot.ORDER:
            self.assertEqual({item["order"].index(label) for item in schedule}, set(range(len(pilot.ORDER))))
        a = dict.fromkeys((*pilot.CLOCKS, "drain_last_write_ms"), 100)
        b = dict.fromkeys(a, 112)
        candidate = dict.fromkeys(a, 87)
        arms = {"fixed2-a": a, "fixed2-b": b, "fixed4": candidate}
        self.assertTrue(pilot.gate_for(arms)["passes"])
        candidate["middle_820_ms"] = 88
        self.assertFalse(pilot.gate_for(arms)["passes"])
        candidate["middle_820_ms"] = 87
        candidate["submit_ms"] = 88
        self.assertFalse(pilot.gate_for(arms)["passes"])
        arms["fixed2-b"] = dict.fromkeys(a, 102)
        candidate["submit_ms"] = 95
        self.assertFalse(pilot.gate_for(arms)["passes"])

    def test_staging_matches_released_peer_arms(self):
        pilot.check_stage_pins(self.config, self.gate)
        for label in pilot.dense.PEERS:
            engine = self.config["lanes"][label]["engine"]
            folder = self.attempt / f"measured-{label}" / f"measured-{engine}"
            score = pilot.score_arm(folder, "measured", label, self.config,
                                    pilot.run.REMOTE / self.series["remotes"][label])
            self.assertGreater(score["middle_820_ms"], 0)
            self.assertGreater(score["submit_ms"], score["submit_last_write_ms"])
            self.assertIn("sql_calls", score)

    def test_fixed_two_receipt_cannot_pass_as_fixed_four(self):
        label = "d2-on"
        folder = self.attempt / "measured-d2-on/measured-mesh"
        inv = pilot.old_inventory(label)
        old = pilot.scorer(2).raw_score(folder, "measured", label, self.config, inv)
        self.assertEqual(old["peak_connections"], 41)
        with self.assertRaisesRegex(ValueError, "runner child options"):
            pilot.scorer(4).raw_score(folder, "measured", label, self.config, inv)

    def test_missing_active_controller_grant_is_offline_only(self):
        with TemporaryDirectory(dir=pilot.dense.ROOT / "target") as path:
            old_queue = pilot.dense.QUEUE
            try:
                pilot.dense.QUEUE = Path(path)
                (Path(path) / "STATUS.md").write_text("TIMING a-different-booking 2000-01-01T00:00:00Z 2099-01-01T00:00:00Z\n")
                with self.assertRaisesRegex(ValueError, "no active sole-controller"):
                    pilot.grant()
            finally:
                pilot.dense.QUEUE = old_queue


if __name__ == "__main__":
    unittest.main()
