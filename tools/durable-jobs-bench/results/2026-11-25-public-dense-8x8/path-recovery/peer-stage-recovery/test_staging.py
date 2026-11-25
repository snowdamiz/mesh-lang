"""Run both peer stagers entirely offline with a recording transport."""
from datetime import datetime, timedelta, timezone
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import peer_recovery as repair


class Staging(unittest.TestCase):
    def dispatch(self, scope, qualification, tmp_name, original_fails):
        root = Path(tmp_name) / "controller"
        root.mkdir()
        access = root / "target/linode-jobs-access"
        access.mkdir(parents=True)
        for name in ("known_hosts", "id_ed25519"):
            (access / name).touch()
        queue = root / "target/linode-jobs-stepchange/requests"
        queue.mkdir(parents=True)
        transfer = repair.original.TRANSFER
        repair.run.write_json(queue / f"{transfer}.released.json",
                              dict(id=transfer, exit_status=0, request_sha256=repair.original.TRANSFER_SEAL))
        lock = root / "tools/durable-jobs-bench/.timing.lock"
        lock.parent.mkdir(parents=True)
        lock.touch()

        dispatched = []
        inventory_bytes = {}
        grant = {"end": (datetime.now(timezone.utc) + timedelta(minutes=10)).isoformat()}
        scope.update(
            OUT=Path(tmp_name) / "evidence",
            active=lambda *_: grant,
            score=lambda *_: {},
        )
        if original_fails:
            scope["replay"] = lambda *_: {"advance_to_peers": True}

        def stage(path, remote, _):
            if path.name == "inventory.json":
                inventory_bytes[remote] = path.read_bytes()

        def dispatch_arm(folder, remote, block, phase, engine):
            dispatched.append((block["group"], folder.name, phase, engine))
            assert remote in inventory_bytes

        ssh = lambda *_, **__: subprocess.CompletedProcess([], 0)
        with (patch.object(repair.run, "SSH", []), patch.object(repair.run, "ssh", ssh),
              patch.object(repair.run, "stage", stage),
              patch.object(repair.run, "dispatch_arm", dispatch_arm)):
            if original_fails:
                with self.assertRaisesRegex(FileNotFoundError, "peer-inventories/river.json"):
                    scope["dispatch"]("peers", "offline", root, qualification)
                self.assertEqual(dispatched, [])
                return
            scope["dispatch"]("peers", "offline", root, qualification)

        trials = repair.original.TRIALS["peers"]
        expected = [(trial["group"], f"{phase}-{label}", phase, repair.original.engine(label))
                    for trial in trials for label in trial["order"] for phase in trial["phases"]]
        self.assertEqual(len(expected), 24)
        self.assertEqual(dispatched, expected)
        for label in repair.original.ENGINES:
            stock = repair.original.HERE / "peer-inventories" / f"{label}.json"
            self.assertEqual(inventory_bytes[repair.remote("peers", label)], stock.read_bytes())
        self.assertEqual(len(inventory_bytes), 5)

    def test_original_error_and_complete_recovered_dispatch(self):
        qualification = repair.check()
        for scope, failure in ((repair.prior.driver(), True), (repair.driver(), False)):
            with tempfile.TemporaryDirectory(prefix=".peer-staging.", dir=repair.original.ROOT) as tmp:
                self.dispatch(scope, qualification, tmp, failure)


if __name__ == "__main__":
    unittest.main()
