"""Check dispatch order offline and reject a partial rotation in combined scoring."""
from datetime import datetime, timedelta, timezone
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import continuation as c


class Continuation(unittest.TestCase):
    def test_host_free_exact_reverse_and_partial_refusal(self):
        qualification, forward = c.check()
        with self.assertRaises(AssertionError):
            c.combine(forward, {key: value for key, value in forward.items() if key != "measured-B"})
        with self.assertRaises(AssertionError):
            c.combine(forward, forward)

        scope = c.driver()
        grant = {"end": (datetime.now(timezone.utc) + timedelta(minutes=10)).isoformat()}
        inventories, dispatched = {}, []
        with tempfile.TemporaryDirectory(prefix=".reverse-stage.", dir=c.original.ROOT) as tmp:
            root = Path(tmp)
            access = root / "target/linode-jobs-access"
            access.mkdir(parents=True)
            (access / "known_hosts").touch()
            (access / "id_ed25519").touch()
            queue = root / "target/linode-jobs-stepchange/requests"
            queue.mkdir(parents=True)
            transfer = c.original.TRANSFER
            c.run.write_json(queue / f"{transfer}.released.json",
                             dict(id=transfer, request_sha256=c.original.TRANSFER_SEAL, exit_status=0))
            lock = root / "tools/durable-jobs-bench/.timing.lock"
            lock.parent.mkdir(parents=True)
            lock.touch()
            scope.update(OUT=root / "evidence", active=lambda *_: grant, score=lambda *_: {})

            def stage(path, remote, _):
                if path.name == "inventory.json":
                    inventories[remote] = path.read_bytes()

            def dispatch_arm(folder, remote, block, phase, engine):
                assert remote in inventories
                dispatched.append((block["group"], folder.name, phase, engine))

            with (patch.object(c.run, "SSH", []),
                  patch.object(c.run, "ssh", lambda *_, **__: subprocess.CompletedProcess([], 0)),
                  patch.object(c.run, "stage", stage), patch.object(c.run, "dispatch_arm", dispatch_arm)):
                scope["dispatch"]("peers", "offline", root, qualification)

        self.assertEqual(len(dispatched), 12)
        self.assertEqual(dispatched, [(2, f"{phase}-{label}", phase, c.original.engine(label))
                                      for label in c.REVERSE["order"] for phase in c.REVERSE["phases"]])
        self.assertEqual(len(inventories), 5)
        for label in c.original.ENGINES:
            self.assertEqual(inventories[c.remote("peers", label)],
                             (c.original.HERE / "peer-inventories" / f"{label}.json").read_bytes())


if __name__ == "__main__":
    unittest.main()
