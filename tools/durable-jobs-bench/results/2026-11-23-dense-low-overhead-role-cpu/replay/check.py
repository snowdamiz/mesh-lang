"""Exercise adverse replay inputs using the preserved genuine Linux trace."""
import copy
from pathlib import Path
import sys
import unittest

from replay import MAX_FRAME_NS, bounds, fixture_windows, read_serial, replay, roster, validate_samples


class TraceCase(unittest.TestCase):
    def test_holds_bad_controls_and_reports_missing_roles(self):
        result = replay(self.entries)
        self.assertFalse(result["control_gates"]["mesh"]["drain_valid"])
        self.assertFalse(result["control_gates"]["peer"]["drain_valid"])
        by_label = {row["label"]: row for row in result["rows"]}
        self.assertIn("contradiction", by_label["mesh-2-on"]["budgets"]["drain"])
        self.assertEqual(by_label["peer-2-on"]["budgets"]["drain"]["error"],
                         "frame spans over 25 ms in the phase bracket")
        churn = by_label["mesh-churn"]
        self.assertIsNone(churn["force"]["missing_transient_cpu"])
        self.assertIsNone(churn["budgets"]["drain"]["captured_pid_cpu_ms_per_job"]["pg_background"])
        self.assertIsNone(churn["budgets"]["drain"]["qualified_role_cpu_ms_per_job"]["pg_jobs"])

    def test_equal_pid_with_new_backend_start_is_reuse(self):
        mesh = copy.deepcopy(self.entries["mesh-2-on"])
        row = next(x for x in mesh["after"]["postgres"]["rows"] if x["login"] == "compiled_caller")
        pid = row["pid"]
        row["start"] = "different lifetime"
        common, coverage = roster(mesh)
        self.assertNotIn(pid, common)
        self.assertIn(pid, coverage["retired_pids"])
        self.assertIn(pid, coverage["late_pids"])
        self.assertIn(pid, coverage["reused_pids"])
        self.assertFalse(coverage["clean_snapshot_screen"])

    def test_denied_clock_emits_no_cpu_and_cannot_recover(self):
        mesh = copy.deepcopy(self.entries["mesh-churn"])
        i = next(i for i, r in enumerate(mesh["before"]["roster"]) if r["role"] == "pg_application")
        for frame in mesh["frames"]:
            r = frame["process"][i]
            r[1], r[3] = 0, 13
        result = bounds(mesh, fixture_windows(mesh))
        self.assertEqual(result["cadence"]["error_codes"]["13"], len(mesh["frames"]))
        self.assertIn(mesh["before"]["roster"][i]["pid"], result["budgets"]["drain"]["process_errors"])
        self.assertFalse(result["budgets"]["drain"]["snapshot_screen_only"])
        mesh["frames"][-1]["process"][i][3] = 0
        with self.assertRaisesRegex(ValueError, "recovered after lifetime or permission loss"):
            validate_samples(mesh["frames"], len(mesh["before"]["roster"]))

    def test_gate_never_admits_slow_frame(self):
        self.assertEqual(MAX_FRAME_NS, 25_000_000)
        peer = self.entries["peer-2-on"]
        self.assertTrue(any(f["end"][2] - f["begin"][0] > MAX_FRAME_NS for f in peer["frames"]))


if __name__ == "__main__":
    serial = sys.argv.pop(1)
    TraceCase.entries = read_serial(Path(serial))
    unittest.main()
