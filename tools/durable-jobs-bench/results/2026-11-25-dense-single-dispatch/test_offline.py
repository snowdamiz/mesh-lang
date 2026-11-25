"""Check the topology reducer against the archived real public hot ledger."""
import json
import tarfile
import unittest

import driver


class ReplayTests(unittest.TestCase):
    def test_archived_hot(self):
        cfg, _ = driver.check()
        historical = driver.retry.OUT / "native"
        replay = driver.run.read_json(driver.ORIGINAL / "evidence/final-replay.json")
        for block in replay["blocks"].values():
            folder = historical / "blocks" / block["attempt"] / "measured-hot/measured-mesh"
            with tarfile.open(folder / "raw.tar.gz") as archive:
                score = driver.public_score(
                    lambda name: driver.run.file_json(archive, f"adapter/{name}.json"),
                    lambda name: driver.dense.text(archive, f"adapter/{name}"),
                    "measured", cfg, 2)
                reference = block["scores"]["measured-hot"]
                self.assertEqual(score, {key: reference[key] for key in score})
                with self.assertRaises(ValueError):
                    driver.public_score(
                        lambda name: driver.run.file_json(archive, f"adapter/{name}.json"),
                        lambda name: driver.dense.text(archive, f"adapter/{name}"),
                        "measured", cfg, 1)

    def test_strict_faster_duplicate_and_spread(self):
        self.assertFalse(driver.clock(1000, 950, 1000)["passes"])
        self.assertTrue(driver.clock(1000, 949, 1000)["passes"])
        self.assertFalse(driver.clock(1000, 900, 1100)["passes"])
        self.assertTrue(driver.clock(1000, 899, 1100)["passes"])

    def test_pinned_launcher_receipt(self):
        receipt = json.loads((driver.HERE / "evidence/launcher.json").read_text())
        self.assertFalse(receipt["workload_ran"])
        for name, pin in receipt["sha256"].items():
            self.assertEqual(driver.run.digest(driver.HERE / name), pin)


if __name__ == "__main__":
    unittest.main()
