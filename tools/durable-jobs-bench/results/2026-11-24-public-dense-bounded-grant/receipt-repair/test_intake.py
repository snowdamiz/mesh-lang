"""Run the original coordinator receipt writer with the host and journal isolated."""
import json
from pathlib import Path
import runpy
import sys
import tempfile
from types import SimpleNamespace
import unittest

from intake import load, PACKET, REQUEST


class IntakeTests(unittest.TestCase):
    def exercise(self, repaired, peer=False):
        driver = load(repaired)
        controller = driver["STATE"] / "completion-writer-r2.py"
        measure = runpy.run_path(str(controller))["measure"]
        with tempfile.TemporaryDirectory(dir=PACKET.parents[3] / "target") as temp:
            root = Path(temp)
            records = []
            measure.__globals__.update(probe=lambda: None, note=records.append, QUEUE=root)
            driver.update(QUEUE=root, runpy=SimpleNamespace(run_path=lambda _: {"measure": measure}),
                          replay=lambda _: {"next_peer_gate": {"passed": True}})
            read = driver["read"]
            packet = read(driver["PACKET"])
            packet.update(evidence_destination=temp, controller_receipt_dir=temp,
                          timing_command=f'"{sys.executable}" -B -c pass',
                          peer_command=f'"{sys.executable}" -B -c pass')
            driver["read"] = lambda path: packet.copy() if path == driver["PACKET"] else read(path)
            status = driver["intake"](None, REQUEST, peer)
            self.assertEqual(status, 0)
            key, numbers = (driver["PEER_ID"], list(range(7, 13))) if peer else (driver["ID"], list(range(7)))
            release = json.loads((root / f"{key}-groups-{numbers[0]}-{numbers[-1]}.released.json").read_text())
            self.assertEqual(release["groups"], numbers)
            self.assertEqual(release["exit_status"], 0)
            self.assertTrue(records[-1].startswith("RELEASED "))
            self.assertEqual(driver["intake"](None, REQUEST, peer), 0)
            self.assertEqual(len(records), 2)

    def test_original_receipt_fails(self):
        with self.assertRaisesRegex(TypeError, "range.*JSON"):
            self.exercise(False)

    def test_primary_and_peer_receipts(self):
        for peers in (False, True):
            with self.subTest(peers=peers):
                self.exercise(True, peers)


if __name__ == "__main__":
    unittest.main()
