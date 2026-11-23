"""Exercise full offline replay and fail-closed evidence checks with synthetic records."""

from contextlib import redirect_stderr, redirect_stdout
from copy import deepcopy
import io
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest

import run
from verify import ENGINES, PROFILES, SUITE, expected_inputs, plan, reconcile
from worker import safe_bytes


ARTIFACT = {"version": "fixture", "artifact_sha256": "a" * 64}
CONFIG = {"cpus": [0, 1], "engines": {engine: {**ARTIFACT, "argv": [f"/bench/{engine}"],
          "postgres_storage": "/bench/pgdata", "evidence_globs": ["*.json", "runner-*.log"]} for engine in ENGINES}}


def ledger(profile, engine):
    runners, slots, cap, digest, _ = PROFILES[profile]
    database = dict.fromkeys(("tls", "fsync", "synchronous_commit", "full_page_writes", "track_commit_timestamp"), True)
    database.update(version="17.11", max_connections=cap, database_connection_limit=cap,
                    reserved_connections=0, superuser_reserved_connections=0)
    example = {**ARTIFACT, "engine": engine, "profile": profile, "input_sha256": digest,
               "runners": runners, "slots": slots, "peak_executing": 1, "peak_connections": cap,
               "database": database, "runner_cpu_seconds": 1.5, "peak_runner_rss_bytes": 1_000_000,
               "transactions": [], "jobs": []}
    for batch, (request, jobs) in enumerate(expected_inputs(profile).items()):
        submitted = 100_000_000 + batch * 500_000
        committed = submitted + 1000
        transaction = {"request_id": request, "submitted_us": submitted, "committed_us": committed,
                       "xid": str(300 + batch), "enqueued_ids": {}}
        example["transactions"].append(transaction)
        for key, (payload, hold, fail) in jobs.items():
            index = len(example["jobs"])
            job_id = str(500 + index)
            transaction["enqueued_ids"][key] = job_id
            example["jobs"].append({"job_key": key, "id": job_id, "payload_sha256": payload,
                "hold_ms": hold, "fail_first": fail, "state": "completed", "attempt": 1, "attempts": 1,
                "effects": 1, "entered_us": committed + 100, "attempt_xid": str(600 + 2 * index),
                "effect_xid": str(601 + 2 * index), "attempt_committed_us": committed + 200,
                "effect_committed_us": committed + 400 + hold * 1000,
                "settled_us": committed + 1500 + hold * 1000})
    return example


def add_bytes(archive, name, content):
    member = tarfile.TarInfo(name)
    member.size = len(content)
    archive.addfile(member, io.BytesIO(content))


def create_block(out, block):
    block_path = out / "blocks" / (run.block_prefix(block) + ".01")
    block_path.mkdir(parents=True)
    run.write_json(block_path / "order.json", block)
    for phase in block["phases"]:
        for engine in block["order"]:
            path = block_path / f"{phase}-{engine}"
            path.mkdir()
            request = {"attempt": "a" * 32, "phase": phase, "engine": engine, "profile": block["profile"]}
            run.write_json(path / "request.json", request)
            samples = [{"epoch_us": epoch, "monotonic_ns": epoch * 1000, "builders": [],
                        "cpu": {"cpu0": [index] * 7 + [0], "cpu1": [index] * 7 + [0]}}
                       for index, epoch in enumerate(range(99_000_000, 118_000_001, 200_000))]
            with tarfile.open(path / "raw.tar.gz", "w:gz") as archive:
                for name, doc in {"invocation.json": request, "status.json": {"exit_code": 0},
                        "host.json": {"affinity": [0, 1], "boot_id": "synthetic", "kernel": "linux",
                                      "pg_mount": {"maj:min": "8:1"}},
                        "adapter/ledger.json": ledger(block["profile"], engine)}.items():
                    add_bytes(archive, name, json.dumps(doc).encode())
                add_bytes(archive, "host-samples.jsonl", "\n".join(map(json.dumps, samples)).encode())
    return block_path


class MatrixTests(unittest.TestCase):
    def test_frozen_plan_and_reversed_controls(self):
        blocks = plan("baseline")
        controls = plan("controls")
        self.assertEqual(len(blocks), 25)
        self.assertEqual(len(controls), 10)
        for profile in PROFILES:
            self.assertEqual(len(expected_inputs(profile)), 24 if profile == "sparse" else 1)
            orders = [b["order"] for b in blocks if b["profile"] == profile]
            self.assertEqual({order[0] for order in orders[:4]}, set(ENGINES))
            reference, reversed_order = [b["order"] for b in controls if b["profile"] == profile]
            self.assertEqual(reference[::-1], reversed_order)
        run.check_inventory(CONFIG)

    def test_two_windows_have_a_common_durable_end(self):
        example = ledger("short", "mesh")
        result = reconcile(example, "mesh", "short", ARTIFACT)
        self.assertEqual(result["completed"], 128)
        self.assertEqual(result["last_settlement_us"], 100_003_500)
        self.assertAlmostEqual(result["commit_jobs_per_s"], 128 / 0.0025)
        self.assertAlmostEqual(result["submit_jobs_per_s"], 128 / 0.0035)

    def test_rejects_unperformed_or_incomparable_work(self):
        for defect in ("missing job", "duplicate xid", "same commit time", "changed payload", "quiet TLS failure"):
            with self.subTest(defect=defect):
                example = ledger("slow", "rust")
                job = example["jobs"][0]
                if defect == "missing job":
                    example["jobs"].pop()
                elif defect == "duplicate xid":
                    example["jobs"][1]["effect_xid"] = job["attempt_xid"]
                elif defect == "same commit time":
                    job["effect_committed_us"] = job["attempt_committed_us"]
                elif defect == "changed payload":
                    job["payload_sha256"] = "0" * 64
                else:
                    example["database"]["tls"] = False
                with self.assertRaises(ValueError):
                    reconcile(example, "rust", "slow", ARTIFACT)

    def test_partial_matrix_produces_no_csv_then_full_replay_produces_five_per_cell(self):
        with tempfile.TemporaryDirectory(dir=SUITE) as temporary:
            path = Path(temporary)
            blocks = run.initialize(path, "baseline", CONFIG)["plan"]
            for block in blocks[:-1]:
                create_block(path, block)
            stdout, stderr = io.StringIO(), io.StringIO()
            with redirect_stdout(stdout), redirect_stderr(stderr):
                self.assertEqual(run.summarize(path), 1)
            self.assertEqual(stdout.getvalue(), "")
            self.assertEqual(json.loads(stderr.getvalue())["missing_blocks"], [run.block_prefix(blocks[-1])])
            create_block(path, blocks[-1])
            with redirect_stdout(stdout), redirect_stderr(io.StringIO()):
                self.assertEqual(run.summarize(path), 0)
            cells = stdout.getvalue().splitlines()
            self.assertEqual(len(cells), 1 + 5 * 4 * 11)
            self.assertTrue(all(row.split(",")[3] == "5" for row in cells[1:]))

    def test_busy_host_and_steal_reject_an_arm(self):
        host = {"affinity": [0, 1]}
        result = {"first_submit_us": 10, "last_settlement_us": 20}
        idle = [{"epoch_us": t, "monotonic_ns": t * 1000, "builders": [],
                 "cpu": {"cpu0": cpu, "cpu1": cpu}}
                for t, cpu in ((0, [0] * 8), (30, [20, 0, 0, 80, 0, 0, 0, 4]))]
        with self.assertRaisesRegex(ValueError, "CPU steal"):
            run.sample_window(host, idle, result)
        busy = deepcopy(idle)
        busy[-1]["builders"] = [[23, "rustc"]]
        with self.assertRaisesRegex(ValueError, "build process"):
            run.sample_window(host, busy, result)

    def test_sanitized_export_removes_uri_and_private_key_material(self):
        with tempfile.TemporaryDirectory(dir=SUITE) as temporary:
            path = Path(temporary)
            proof = path / "proof.json"
            proof.write_text(json.dumps({"tls": True, "dsn": "postgresql://user:opaque@127.0.0.1/db",
                                         "password": "opaque", "version": "17.11"}))
            self.assertEqual(json.loads(safe_bytes(proof)), {"tls": True, "dsn": "[redacted]", "version": "17.11"})
            log = path / "wrapper.log"
            log.write_text("-----BEGIN OPENSSH PRIVATE KEY-----\nopaque\n-----END OPENSSH PRIVATE KEY-----")
            self.assertNotIn(b"opaque", safe_bytes(log))

    def test_failed_worker_invocation_can_be_exported_without_starting_an_adapter(self):
        with tempfile.TemporaryDirectory(dir=SUITE) as temporary:
            series = Path(temporary) / "series"
            series.mkdir()
            driver = series / "worker.py"
            shutil.copyfile(SUITE / "worker.py", driver)
            run.write_json(series / "inventory.json", {**CONFIG, "cpus": [10_000, 10_001]})
            command = [sys.executable, "-B", driver]
            attempt = "e" * 32
            result = subprocess.run([*command, "run", attempt, "--engine", "mesh", "--profile", "sparse",
                                     "--phase", "measured"], capture_output=True, check=False)
            self.assertEqual(result.returncode, 1)
            exported = subprocess.check_output([*command, "export", attempt])
            with tarfile.open(fileobj=io.BytesIO(exported), mode="r:gz") as archive:
                self.assertEqual(run.file_json(archive, "invocation.json")["attempt"], attempt)
                self.assertEqual(run.file_json(archive, "status.json")["exit_code"], 1)
                self.assertIn("native comparison requires", run.file_json(archive, "error.json")["error"])
                self.assertNotIn("adapter/ledger.json", archive.getnames())


if __name__ == "__main__":
    unittest.main()
