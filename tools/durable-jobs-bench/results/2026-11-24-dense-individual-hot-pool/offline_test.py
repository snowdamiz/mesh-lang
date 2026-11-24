"""Exercise the new transaction and clock checks on real historical ledgers."""
import json
import tarfile
import unittest

import controller
from score import intervals, lifecycle


class HistoricalControls(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.root = controller.dense.HERE / "evidence/native-closure-retry"
        cls.blocks = json.loads((cls.root / "final-replay.json").read_text())["blocks"]

    def raw(self, block, phase, label):
        path = self.root / "blocks" / block["attempt"] / f"{phase}-{label}"
        raw, = path.glob("*/raw.tar.gz")
        return tarfile.open(raw)

    def test_grouped_mesh_is_rejected(self):
        for block in self.blocks.values():
            with self.raw(block, "measured", "hot") as archive:
                ledger = controller.run.file_json(archive, "adapter/ledger.json")
                with self.assertRaisesRegex(ValueError, "individual terminal transactions"):
                    lifecycle(ledger, individual=True)

    def test_historical_peer_clocks_and_transactions(self):
        for block in self.blocks.values():
            for label in controller.PEERS:
                for phase in ("warmup", "measured"):
                    with self.raw(block, phase, label) as archive:
                        ledger = controller.run.file_json(archive, "adapter/ledger.json")
                        lifecycle(ledger, individual=False)
                        clocks = intervals(ledger)
                        saved = block["scores"][f"{phase}-{label}"]
                        self.assertAlmostEqual(1_024_000_000 / clocks["submit_us"], saved["submit_jobs_per_s"])
                        self.assertAlmostEqual(1_024_000_000 / clocks["drain_us"], saved["commit_jobs_per_s"])
                        self.assertGreater(clocks["middle_effect_us"], 0)
                        self.assertGreater(clocks["middle_settlement_us"], 0)


if __name__ == "__main__":
    unittest.main()
