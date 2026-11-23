"""Negative controls for the archived Apalis observations."""

import json
import unittest

import replay


class ReplayTests(unittest.TestCase):
    def setUp(self):
        files = replay.read_archive("functional-v1")
        self.jobs = json.loads(files["adapter/raw-jobs.json"])
        self.transactions = json.loads(files["adapter/transactions.json"])["committed"]
        self.writes = json.loads(files["adapter/raw-writes.json"])
        self.pids = {row["worker_pid"] for row in self.writes["attempts"]}
        self.database = json.loads(files["adapter/database.json"])

    def verify_rows(self):
        return replay.verify_rows(self.jobs, self.transactions, self.writes, self.pids)

    def test_rejects_duplicate_application_and_product_rows(self):
        for category in ("product", "attempts", "effects"):
            with self.subTest(category=category):
                rows = self.writes[category]
                rows.append(rows[0].copy())
                with self.assertRaisesRegex(ValueError, f"(incorrect durable {category}|product rows)"):
                    self.verify_rows()
                rows.pop()

    def test_rejects_missing_second_attempt(self):
        self.writes["attempts"] = [row for row in self.writes["attempts"] if row["attempt"] != 2]
        with self.assertRaisesRegex(ValueError, "incorrect durable attempts"):
            self.verify_rows()

    def test_rejects_duplicate_native_job_row(self):
        self.jobs.append(self.jobs[0].copy())
        with self.assertRaisesRegex(ValueError, "native job rows and enqueue receipt identities differ"):
            self.verify_rows()

    def test_rejects_native_id_attached_to_wrong_request(self):
        first, second = (tx["enqueued_ids"] for tx in self.transactions[:2])
        first_key, second_key = next(iter(first)), next(iter(second))
        first[first_key], second[second_key] = second[second_key], first[first_key]
        with self.assertRaisesRegex(ValueError, "native job rows and enqueue receipt identities differ"):
            self.verify_rows()

    def test_rejects_raw_write_from_wrong_native_id(self):
        self.writes["attempts"][0]["job_id"] = self.jobs[1]["id"]
        with self.assertRaisesRegex(ValueError, "unexpected application write identity"):
            self.verify_rows()

    def test_rejects_reused_application_xid(self):
        self.writes["effects"][0]["xid"] = self.writes["attempts"][0]["xid"]
        with self.assertRaisesRegex(ValueError, "reused application transaction XID"):
            self.verify_rows()

    def test_rejects_wrong_observed_postgres_version(self):
        self.database["version"] = "17.10"
        with self.assertRaisesRegex(ValueError, "expected PostgreSQL 17.11"):
            replay.database_check(self.database, 5)


if __name__ == "__main__":
    unittest.main()
