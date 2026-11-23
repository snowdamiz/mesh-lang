"""Negative controls for the offline native-02 reduction."""
from copy import deepcopy
import json
from pathlib import Path
import tarfile
from unittest import TestCase, main

from recover import (
    archived_processes, host_projection, lines, replay, verify_frame_order,
)

HERE = Path(__file__).resolve().parent.parent
SERIES = HERE / "evidence/native-02"
CPUS = ["cpu0", "cpu1"]


def on(order, lane, engine):
    return tarfile.open(SERIES / order / lane / "02-on" / f"diagnostic-{engine}" / "raw.tar.gz")


class RecoveryTests(TestCase):
    def test_all_arms_and_original_qualification(self):
        data = replay(SERIES)
        self.assertEqual(len(data["arms"]), 18)
        self.assertTrue(all(row["score"]["completed"] == 1024 for row in data["arms"]))
        probes = [row for row in data["arms"] if row["mode"] == "on"]
        self.assertEqual(sum("independent_fine_host" in row["host_windows"]["drain"] for row in probes), 6)
        clean = [row for row in data["arms"] if row["mode"] == "off"]
        bounds = {lane: [row["host_windows"]["drain"]["canonical_sparse_host"]["host_cpu_ms_per_job"]["busy"]
                        for row in clean if row["lane"] == lane] for lane in ("ordinary", "oban", "river")}
        self.assertGreater(min(lo for lo, _ in bounds["ordinary"]), max(hi for _, hi in bounds["oban"]))
        self.assertGreater(min(lo for lo, _ in bounds["ordinary"]), max(hi for _, hi in bounds["river"]))
        original = data["qualification"]
        recovered = data["host_only_matched_controls_same_gates"]
        self.assertEqual({k for k, item in original.items() if item["drain_valid"]}, {"river"})
        self.assertEqual({k for k, item in recovered.items() if item["drain_valid"]}, {"river"})
        self.assertEqual([p["valid"] for p in recovered["ordinary"]["pairs"]], [True, False])
        self.assertTrue(all(not p["phases"]["drain"]["valid"] for p in recovered["oban"]["pairs"]))

    def test_bad_roster_cannot_erase_valid_host_evidence(self):
        with on("forward", "ordinary", "mesh") as archive:
            _, *frames = lines(archive, "role-frames.jsonl")
        old = host_projection(frames, CPUS)
        broken = deepcopy(frames)
        for frame in broken:
            frame["roster_error"] = "synthetic roster failure"
        self.assertEqual(old, host_projection(broken, CPUS))

    def test_bad_host_does_not_gain_bounds_from_role_evidence(self):
        with on("forward", "oban", "oban") as archive:
            _, *frames = lines(archive, "role-frames.jsonl")
        broken = deepcopy(frames)
        broken[1]["host"]["ticks"]["cpu0"][0] = 0
        with self.assertRaisesRegex(ValueError, "host CPU counter moved backwards"):
            host_projection(broken, CPUS)
        broken = deepcopy(frames)
        broken[1]["begin"][1] = broken[0]["begin"][1] - 1
        with self.assertRaisesRegex(ValueError, "capture clocks moved backwards"):
            verify_frame_order(broken)

    def test_partial_rows_excluded_and_identity_drift_rejected(self):
        with on("forward", "oban", "oban") as archive:
            _, *frames = lines(archive, "role-frames.jsonl")
            def parse(name):
                return json.load(archive.extractfile("adapter/" + name + ".json"))
            census = parse("postgres-tls-sessions")
            masks = parse("process-affinity")
            broker = parse("oban-broker-emulator")["pid"]
            used = {str(job["worker_pid"]) for job in parse("ledger")["jobs"]}
        args = census, masks, broker, used, [0, 1]
        _, _, complete = archived_processes(frames, *args)
        self.assertEqual(len(complete), 179)
        self.assertEqual(sum("processes" in frame and "roster_error" in frame for frame in frames), 1)
        broken = deepcopy(complete[1])
        row = next(iter(broken["processes"].values()))
        row["start"] += 1
        with self.assertRaisesRegex(ValueError, "changed identity"):
            archived_processes([complete[0], broken], *args)


if __name__ == "__main__":
    main()
