"""Exercise the controller journal boundary without starting a workload."""
from datetime import datetime, timezone
from pathlib import Path
import runpy
import unittest

driver = runpy.run_path(str(Path(__file__).with_name("grant_recovery.py")))
active_window = driver["active_window"]
identifier = driver["identifier"]


class ControllerGrant(unittest.TestCase):
    def test_current_grant_only(self):
        now = datetime(2026, 11, 24, 22, 29, 21, tzinfo=timezone.utc)
        valid = f"TIMING {identifier} 2026-11-24T22:29:20Z 2026-11-24T22:53:20Z"
        self.assertEqual(active_window("HOLD matrix-controller\n" + valid, now), tuple(valid.split()[2:]))
        for journal in (valid + "\nHOLD matrix-controller", valid + "\nRELEASED prior 22:29 UTC",
                        valid.replace("TIMING", "SETUP"), valid.replace("29:20", "39:20"),
                        valid.replace("53:20", "29:21"), valid.replace(identifier, "another-series"),
                        valid.rsplit(" ", 1)[0], "", valid + " unexpected"):
            with self.subTest(journal=journal), self.assertRaises(ValueError):
                active_window(journal, now)


if __name__ == "__main__":
    unittest.main()
