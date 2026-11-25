import tarfile
import unittest

from driver import INITIAL_OUT, LOCK, report_errors


class Recovery(unittest.TestCase):
    def test_exact_probe_and_runner_errors(self):
        files = sorted((INITIAL_OUT/"blocks").glob("00-noop.*/measured-plan-*/measured-mesh/raw.tar.gz"))
        self.assertEqual(len(files), 2)
        for path in files:
            with self.subTest(arm=path.parents[1].name), tarfile.open(path) as archive:
                log = archive.extractfile("adapter/postgres.log").read().decode()
                result = report_errors(archive, log)
                clone = [row for row in result["errors"] if row["db"] == "dense_grant_probe"]
                measured = [row for row in result["errors"] if row["db"] == "mesh"]
                self.assertEqual(len(clone), 3)
                self.assertEqual(len(measured), int(path.parents[1].name == "measured-plan-stock"))
                probe = clone[0]
                stamp, pid = divmod(probe["at_us"], 1_000_000)[0], probe["pid"]
                old = f"{stamp}.{probe['at_us'] % 1_000_000 // 1_000:03d}|{pid}|"
                late = f"{result['first_observer_us'] // 1_000_000}.999|{pid}|"
                mutations = (
                    log.replace(old, late),
                    log.replace("|dense_grant_probe| ERROR:  " + LOCK, "|mesh| ERROR:  " + LOCK, 1),
                    log.replace("ERROR:  " + LOCK, "ERROR:  deadlock detected", 1),
                    log.replace("STATEMENT:  SELECT coalesce(json_agg", "STATEMENT:  SELECT count", 1),
                )
                for edited in mutations:
                    self.assertNotEqual(edited, log)
                    with self.assertRaises(AssertionError):
                        report_errors(archive, edited)


if __name__ == "__main__":
    unittest.main()
