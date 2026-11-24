"""Exercise the new setting witnesses using original native exports, without dispatch."""
import tarfile
import unittest

import controller as ctl


class GroupCommitTests(unittest.TestCase):
    def test_control_spread_and_floor(self):
        def passes(a, treatment, b):
            arms = [dict(middle_write_ms=value, submit_jobs_per_s=1024000 / value)
                    for value in (a, treatment, b)]
            outcomes = [ctl.clock_gate(key, arms)["passes"] for key in arms[0]]
            self.assertEqual(*outcomes)
            return outcomes[0]

        self.assertFalse(passes(100, 95, 100))
        self.assertFalse(passes(100, 82, 119))
        self.assertFalse(passes(119, 82, 100))
        self.assertTrue(passes(119, 80, 100))

    def test_settings_on_native_frames(self):
        wal = ctl.wal
        config, _ = ctl.check()
        historical = ctl.run.read_json(ctl.PREVIOUS / "evidence/native/final-replay.json")
        block = historical["groups"]["1"]
        original = wal.SETTINGS
        self.addCleanup(setattr, wal, "SETTINGS", original)
        for label in wal.ORDER:
            name = "measured-" + label
            engine = config["lanes"][label]["engine"]
            raw = ctl.PREVIOUS / "evidence/native/blocks" / block["attempt"] / name / ("measured-" + engine) / "raw.tar.gz"
            score = block["arms"][name]
            with tarfile.open(raw) as archive:
                before, after, closure = [ctl.run.file_json(archive, f"adapter/wal-{frame}.json")
                                          for frame in ("before", "live-after", "restored")]
            frames = (before, after, closure["restored"])
            for delay, siblings in (("0", "5"), ("100", "1")):
                wal.SETTINGS = dict(original, commit_delay=delay, commit_siblings=siblings)
                for frame in frames:
                    frame["frame"]["settings"].update(commit_delay=delay, commit_siblings=siblings)
                priced = wal.verify_wal(before, after, closure, score)
                self.assertEqual(priced["whole_workload_delta"], score["postgres_wal"]["whole_workload_delta"])
                for frame in frames:
                    settings = frame["frame"]["settings"]
                    for key in ("commit_delay", "commit_siblings", "synchronous_commit"):
                        saved = settings.pop(key)
                        with self.assertRaisesRegex((AssertionError, ValueError, RuntimeError), "settings"):
                            wal.verify_wal(before, after, closure, score)
                        settings[key] = saved


if __name__ == "__main__":
    unittest.main()
