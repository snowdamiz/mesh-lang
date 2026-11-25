"""Correctness controls with frozen native evidence and synthetic candidate counters."""
from copy import deepcopy
import json
from pathlib import Path
import runpy
import tarfile
from tempfile import TemporaryDirectory
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import controller as ctl


class PacketTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.config, _ = ctl.check()
        prior = ctl.PREVIOUS / "evidence/native"
        block = ctl.run.read_json(prior / "final-replay.json")["groups"]["1"]
        cls.score = block["arms"]["measured-d2-on"]
        cls.raw = prior / "blocks" / block["attempt"] / "measured-d2-on/measured-mesh/raw.tar.gz"
        assert ctl.run.digest(cls.raw) == cls.score["raw_sha256"]
        with tarfile.open(cls.raw) as archive:
            cls.frames = [ctl.run.file_json(archive, f"adapter/wal-{name}.json")
                          for name in ("before", "live-after", "restored")]

    def test_frozen_workload_reconciles(self):
        wal = ctl.wal
        manifest = ctl.run.read_json(ctl.PREVIOUS / "evidence/native/series.json")
        remote = ctl.run.REMOTE / manifest["remotes"]["d2-on"]
        with patch.object(wal, "HERE", ctl.PREVIOUS):
            arm = wal.score_arm(self.raw.parent, "measured", "d2-on", self.config, remote)
        self.assertEqual(arm, self.score)

    def test_frozen_replay_and_method_accounting(self):
        wal = ctl.wal
        for name in ("verify_wal", "device", "score_arm", "SETTINGS"):
            self.addCleanup(setattr, wal, name, getattr(wal, name))
        ctl.install_replay(ctl.STOCK)
        before, after, closure = deepcopy(self.frames)
        frames = [row["frame"] for row in (before, after, closure["restored"])]
        wal.SETTINGS = dict(wal.SETTINGS, commit_siblings="5", debug_io_direct="")
        for frame in frames:
            frame["settings"].update(commit_siblings="5", debug_io_direct="")
        stock = wal.verify_wal(before, after, closure, self.score)["whole_workload_delta"]
        self.assertEqual(stock, self.score["postgres_wal"]["whole_workload_delta"])
        with tarfile.open(self.raw) as archive:
            device = wal.device(archive, self.score, closure["stop_begin"]["epoch_us"])
        self.assertEqual(device["outer"][:2], self.score["disk"]["outer"])
        self.assertEqual(device["inner"][:2], self.score["disk"]["inner"])
        self.assertGreater(device["outer"][2], 0)
        self.assertTrue(all(i <= o for i, o in zip(device["inner"], device["outer"], strict=True)))

        wal.SETTINGS = dict(wal.SETTINGS, wal_sync_method=ctl.CANDIDATE)
        for frame in frames:
            frame["settings"]["wal_sync_method"] = ctl.CANDIDATE
            row = frame["wal"]
            row["wal_write_time"] += row["wal_sync_time"]
            row["wal_sync"] = row["wal_sync_time"] = 0
        candidate = wal.verify_wal(before, after, closure, self.score)["whole_workload_delta"]
        self.assertAlmostEqual(candidate["wal_write_time"], stock["wal_write_time"] + stock["wal_sync_time"])
        self.assertEqual(candidate["wal_sync_time"], 0)
        frames[1]["settings"]["wal_sync_method"] = ctl.STOCK
        with self.assertRaisesRegex(ValueError, "settings"):
            wal.verify_wal(before, after, closure, self.score)
        frames[1]["settings"]["wal_sync_method"] = ctl.CANDIDATE
        frames[2]["wal"]["wal_sync"] = 1
        with self.assertRaisesRegex(ValueError, "missing PostgreSQL WAL I/O charges"):
            wal.verify_wal(before, after, closure, self.score)
        frames[2]["wal"]["wal_sync"] = 0
        for frame in frames:
            frame["wal"]["wal_write"] = 0
        with self.assertRaisesRegex(ValueError, "missing PostgreSQL WAL I/O charges"):
            wal.verify_wal(before, after, closure, self.score)

    def test_mount_and_write_support(self):
        observer = runpy.run_path(str(ctl.STAGES / ctl.CANDIDATE / "wal_capture.py"))
        probe = observer["probe_support"]
        probe_globals = probe.__globals__
        os = probe_globals["os"]
        mount = dict(observer["WAL_MOUNT"], options="rw,relatime")
        with TemporaryDirectory(dir=ctl.STAGES) as root:
            pg_wal = Path(root) / "pg_wal"
            pg_wal.mkdir()
            methods = []
            fdatasync_calls = []

            def open_probe(path, flags, mode):
                methods.append(flags & 0o10000)
                return os.open(path, flags & ~0o10000, mode)

            fake_os = SimpleNamespace(O_DSYNC=0o10000, O_SYNC=0o4010000, O_WRONLY=os.O_WRONLY,
                O_CREAT=os.O_CREAT, O_EXCL=os.O_EXCL, open=open_probe, close=os.close, pwrite=os.pwrite,
                fdatasync=fdatasync_calls.append, major=lambda _: 8, minor=lambda _: 0,
                uname=lambda: SimpleNamespace(release=self.config["host"][1]))
            fake_fcntl = SimpleNamespace(F_GETFL=3, fcntl=lambda *_: methods[-1])
            findmnt = lambda *_args, **_kwargs: json.dumps({"filesystems": [mount]})
            with patch.dict(probe_globals, os=fake_os, fcntl=fake_fcntl,
                            sys=SimpleNamespace(platform="linux"),
                            subprocess=SimpleNamespace(check_output=findmnt)):
                support = probe(Path(root))
                ctl.verify_support(support, ctl.CANDIDATE, self.config["host"][1])
                self.assertEqual(methods, [0, 0o10000])
                self.assertEqual(len(fdatasync_calls), 1)
                self.assertEqual(list(pg_wal.iterdir()), [])
                mount["options"] = "rw,nobarrier"
                with self.assertRaisesRegex(ValueError, "ext4"):
                    probe(Path(root))
                mount["options"] = "rw"
                fake_fcntl.fcntl = lambda *_: 0
                with self.assertRaisesRegex(ValueError, "kernel did not retain"):
                    probe(Path(root))
                self.assertEqual(list(pg_wal.iterdir()), [])

    def test_absolute_and_percent_gates(self):
        self.assertFalse(ctl.clock_gate(100, 95, 100, 0)["passes"])
        self.assertFalse(ctl.clock_gate(100, 85, 115, 0)["passes"])
        self.assertFalse(ctl.clock_gate(115, 85, 100, 0)["passes"])
        self.assertFalse(ctl.clock_gate(115, 84, 100, 16)["passes"])
        self.assertTrue(ctl.clock_gate(115, 83, 100, 16)["passes"])

    def test_replay_requires_all_six_clock_gates(self):
        receipts = []
        for index, (_, case) in enumerate(ctl.schedule("pilot")):
            end = 1_000_000 if case != "candidate" else 850_000
            arm = dict(first_submit_us=1, first_commit_us=100_001, last_settlement_us=end,
                       middle_write_ms=700 if case != "candidate" else 550, raw_sha256="fixture",
                       postgres_wal={"whole_workload_delta": {"wal_write_plus_sync_ms": 10}}, disk={})
            arms = {f"{phase}-d2-on": dict(arm, cluster_id=f"{index}-{phase}",
                        postgres_bracket_us=[4 * index + offset, 4 * index + offset + 1])
                    for phase, offset in (("warmup", 0), ("measured", 2))}
            receipts.append(dict(arms=arms))

        def advances():
            with patch.object(ctl.subprocess, "check_output", side_effect=map(json.dumps, receipts)):
                return ctl.replay("pilot")["advance_to_pricing"]

        self.assertTrue(advances())
        receipts[4]["arms"]["measured-d2-on"]["middle_write_ms"] = 685
        self.assertFalse(advances())

    def test_reject_overlapping_or_reused_clusters(self):
        first = dict(postgres_bracket_us=[1, 2], cluster_id="first")
        second = dict(postgres_bracket_us=[3, 4], cluster_id="second")
        ctl.ordered_arms([first, second])
        with self.assertRaisesRegex(ValueError, "out of order"):
            ctl.ordered_arms([second, first])
        with self.assertRaisesRegex(ValueError, "reuse"):
            ctl.ordered_arms([first, dict(second, cluster_id="first")])

    def test_missing_grant_stops_before_native_dispatch(self):
        with TemporaryDirectory(dir=ctl.STAGES) as root:
            queue = Path(root)
            (queue / "STATUS.md").write_text("TIMING different-campaign 2000-01-01T00:00:00Z 2999-01-01T00:00:00Z\n")
            with patch.object(ctl, "QUEUE", queue), patch.object(ctl.run, "ssh", side_effect=AssertionError):
                with self.assertRaisesRegex(ValueError, "requires a sole-controller"):
                    ctl.grant("pilot")


if __name__ == "__main__":
    unittest.main()
