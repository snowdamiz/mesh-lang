"""Read-only controls for telemetry, lineage and the matched inventories."""
from copy import deepcopy
import json
import tarfile
import unittest

import retry
from logger import verify_dense_logs, verify_runner_log


class RecoveryChecks(unittest.TestCase):
    def test_failed_native_logs_show_live_telemetry(self):
        a, b = retry.failure_analysis()
        self.assertEqual([a["lines"], b["lines"]], [1008, 1048])
        self.assertEqual([a["claimed"], b["claimed"]], [502, 522])
        self.assertEqual([row["event_buffer_bytes"]["logger"]["sampled_peak_bytes"] for row in (a, b)],
                         [4104, 3876])

    def test_real_logger_off_reference(self):
        root = retry.dense.HERE / "evidence/native-closure-retry"
        blocks = json.loads((root / "final-replay.json").read_text())["blocks"]
        for block in blocks.values():
            path = root / "blocks" / block["attempt"] / "measured-hot"
            raw, = path.glob("*/raw.tar.gz")
            with tarfile.open(raw) as archive:
                verify_dense_logs(lambda name: retry.run.file_json(archive, f"adapter/{name}.json"),
                                  lambda name: retry.dense.text(archive, f"adapter/{name}"))

    def test_logger_reports_and_event_rows_are_independently_rejected(self):
        drained = '{"state":"stopped","drained":true}'
        verify_runner_log(f"READY\n{drained}\n")
        for text in (f'READY\n{{"event":"attempt_started"}}\n{drained}',
                     'READY\n{"state":"stopped","drained":true,"event_buffer_bytes":{"logger":{}}}',
                     f'READY\n{{"event":\n{drained}',
                     "READY"):
            with self.assertRaises(ValueError):
                verify_runner_log(text)

    def test_only_main_and_elf_may_change(self):
        old = retry.old_manifests()["jobs"]
        new = deepcopy(old)
        new["source_sha256"] = "a" * 64
        new["files"].update({"main.mpl": retry.FIXTURE, "mesh": "b" * 64})
        retry.verify_relink_manifest(old, new, retry.FIXTURE, "a" * 64)
        bad_runtime = dict(new, runtime_sha256="c" * 64)
        with self.assertRaisesRegex(ValueError, "exact runtime"):
            retry.verify_relink_manifest(old, bad_runtime, retry.FIXTURE, "a" * 64)
        new["files"]["writes.mpl"] = "c" * 64
        with self.assertRaisesRegex(ValueError, "changed files"):
            retry.verify_relink_manifest(old, new, retry.FIXTURE, "a" * 64)

    def test_same_peer_and_checkout_choice(self):
        old = retry.OLD_BIND()
        config = retry.base.config(old)
        remote = retry.run.REMOTE / "offline-retry"
        peers = {label: retry.base.stages(config, label, remote) for label in retry.base.PEERS}
        original = {field: getattr(retry.base, field) for field in
                    ("ID", "STAGE", "OUT", "ARTIFACT", "request", "adapter_files", "bind_proof", "mesh")}
        retry.configure()
        try:
            for label, inventory in peers.items():
                self.assertEqual(retry.base.stages(config, label, remote), inventory)
            off = retry.base.stages(config, "checked-a", remote)
            self.assertEqual(off, retry.base.stages(config, "checked-b", remote))
            off[1]["engines"]["mesh"]["argv"][1] = "HOT_CHECKOUT=1"
            self.assertEqual(off, retry.base.stages(config, "hot", remote))
        finally:
            for field, value in original.items():
                setattr(retry.base, field, value)


if __name__ == "__main__":
    unittest.main()
