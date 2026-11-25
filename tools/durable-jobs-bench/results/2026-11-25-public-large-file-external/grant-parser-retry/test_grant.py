"""Replay the real controller records without accessing the host or inbox."""

from datetime import datetime
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import matched

USE_ORIGINAL = "--original" in sys.argv
if USE_ORIGINAL:
    sys.argv.remove("--original")
else:
    import retry

JOURNAL = (HERE / "evidence/journal.txt").read_text()
ORIGINAL_ID = matched.ID
ACTIVE, RELEASED = JOURNAL.split(f"RELEASED {ORIGINAL_ID}", 1)
RELEASED, NEWER = RELEASED.split("\n\nTIMING ", 1)
RELEASED = ACTIVE + f"RELEASED {ORIGINAL_ID}" + RELEASED + "\n"
NEWER = RELEASED + "\nTIMING " + NEWER
START = "2026-11-25T02:28:26Z"
END = "2026-11-25T02:52:26Z"


class GrantTest(unittest.TestCase):
    def grant(self, journal, now, *, original_id=None, registered=True):
        driver = matched if USE_ORIGINAL else retry
        packet = driver.PACKET
        request = packet.read_bytes()
        grant_id = original_id or driver.ID
        inbox = driver.QUEUE / f"{grant_id}.json"
        status = driver.QUEUE / "STATUS.md"
        read_bytes = Path.read_bytes

        def request_bytes(path):
            if path == inbox:
                return request if registered else b"changed inbox"
            return read_bytes(path)

        def journal_text(path):
            assert path == status
            return journal

        with patch.object(Path, "read_text", autospec=True, side_effect=journal_text), \
             patch.object(Path, "read_bytes", autospec=True, side_effect=request_bytes), \
             patch.object(Path, "is_file", autospec=True, side_effect=lambda path: path == inbox), \
             patch.object(driver, "datetime", wraps=datetime) as clock:
            clock.now.return_value = datetime.fromisoformat(now)
            with patch.object(driver, "ID", grant_id):
                return driver.grant()

    def test_exact_authorized_row(self):
        result = self.grant(ACTIVE, START, original_id=ORIGINAL_ID)
        self.assertEqual((result["id"], result["start"], result["end"]), (ORIGINAL_ID, START, END))

    def test_retry_identity_and_seal(self):
        if USE_ORIGINAL:
            self.skipTest("original driver has no sibling identity")
        result = self.grant(ACTIVE.replace(ORIGINAL_ID, retry.ID), START)
        self.assertEqual(result, dict(id=retry.ID, start=START, end=END,
                                      request_sha256=matched.sha(retry.PACKET)))

    def test_release_revokes_live_interval(self):
        with self.assertRaisesRegex(ValueError, "requires an active"):
            self.grant(RELEASED, "2026-11-25T02:28:28Z", original_id=ORIGINAL_ID)

    def test_newer_grant_revokes_earlier_interval(self):
        # Remove the release to exercise the newer grant independently.
        latest = ACTIVE + "\nTIMING " + NEWER.rsplit("\nTIMING ", 1)[1]
        with self.assertRaisesRegex(ValueError, "requires an active"):
            self.grant(latest, "2026-11-25T02:31:00Z", original_id=ORIGINAL_ID)

    def test_expiry_and_future_start(self):
        for now in ("2026-11-25T02:28:25Z", END, "2026-11-25T02:52:27Z"):
            with self.subTest(now=now), self.assertRaisesRegex(ValueError, "outside the active"):
                self.grant(ACTIVE, now, original_id=ORIGINAL_ID)

    def test_changed_inbox_is_refused(self):
        with self.assertRaisesRegex(ValueError, "registered request"):
            self.grant(ACTIVE, START, original_id=ORIGINAL_ID, registered=False)


if __name__ == "__main__":
    unittest.main()
