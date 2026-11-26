"""Offline stage and pairing regressions with synthetic inputs and no score exports."""

from argparse import Namespace
from copy import deepcopy
import json
from pathlib import Path
import shutil
import tarfile
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import patch

import analyze_stages as stages
import run as matrix
from test_matrix import ARTIFACT, CONFIG, add_bytes, ledger
from verify import PROFILES, plan, reconcile


def evidence(profile, engine, offset=0):
    result = ledger(profile, engine)
    for tx in result["transactions"]:
        for field in ("submitted_us", "committed_us"):
            tx[field] += offset
        tx["acknowledged_us" if engine in ("mesh", "oban") else "returned_us"] = tx["committed_us"] + 200
        if engine == "go":
            tx.update(read_finished_us=tx["submitted_us"] + 200, decoded_us=tx["submitted_us"] + 400,
                      enqueue_called_us=tx["submitted_us"] + 600, input_bytes=100)
    for index, job in enumerate(result["jobs"]):
        for field in stages.TIMESTAMP_FIELDS:
            job[field] += offset
        job["settled_xid"] = str(10_000 + index)
        if engine != "rust":
            job["claimed_us"] = job["entered_us"] - 100
        else:
            job["last_lock_us"] = job["entered_us"] + 50
    return result


def metrics(example):
    return stages.stage_metrics(example, reconcile(example, example["engine"], example["profile"], ARTIFACT))


def create_block(directory, block, config, position, wrong_adapter_phase=None):
    path = directory / "blocks" / (matrix.block_prefix(block) + ".01")
    path.mkdir(parents=True)
    matrix.write_json(path / "order.json", block)
    for phase_index, phase in enumerate(block["phases"]):
        for order_index, engine in enumerate(block["order"]):
            arm = path / f"{phase}-{engine}"
            arm.mkdir()
            number = 1 + 8 * position + 4 * phase_index + order_index
            request = {"attempt": f"{number:032x}", "phase": phase,
                       "profile": block["profile"], "engine": engine}
            matrix.write_json(arm / "request.json", request)
            data = evidence(block["profile"], engine, number * 30_000_000)
            adapter_phase = "diagnostic" if phase == "control" else phase
            if wrong_adapter_phase == (phase, engine):
                adapter_phase = "diagnostic"
            runtime = {"phase": adapter_phase, "artifact_sha256": data["artifact_sha256"],
                       "submit_boundary": "before_batch_read"}
            if engine in ("mesh", "oban"):
                data.update(phase=adapter_phase, local_smoke=False)
            start = data["transactions"][0]["submitted_us"]
            end = max(job["settled_us"] for job in data["jobs"])
            samples = [{"epoch_us": stamp, "monotonic_ns": stamp * 1000, "builders": [], "diskstats": [],
                        "cpu": {f"cpu{cpu}": [index] * 7 + [0] for cpu in config["cpus"]}}
                       for index, stamp in enumerate(range(start - 1, end + 200_000, 200_000))]
            with tarfile.open(arm / "raw.tar.gz", "w:gz") as archive:
                docs = {"invocation.json": request, "status.json": {"exit_code": 0, "finished_us": end + 300_000},
                        "host.json": {"architecture": "x86_64", "available_cpus": list(range(4)),
                                      "affinity": config["cpus"], "boot_id": "synthetic", "kernel": "linux",
                                      "pg_mount": {"maj:min": "8:1", "fstype": "ext4"},
                                      "verified_files": config["engines"][engine].get("verified_files", {})},
                        "adapter/ledger.json": data, "adapter/runtime.json": runtime}
                for member, doc in docs.items():
                    add_bytes(archive, member, json.dumps(doc).encode())
                add_bytes(archive, "host-samples.jsonl", "\n".join(map(json.dumps, samples)).encode())
    return path


class StageTests(unittest.TestCase):
    def test_apalis_large_settlement_retains_the_one_delayed_job(self):
        example = evidence("large", "rust")
        delayed = example["jobs"][0]
        commit = example["transactions"][0]["committed_us"]
        delayed["settled_us"] = commit + 25_000_000
        scores = reconcile(example, "rust", "large", ARTIFACT)
        detail = stages.apalis_large_settlement(example, scores)

        self.assertEqual(detail["last_job"], {"job_key": delayed["job_key"], "fail_first": False,
                                             "attempt": 1, "attempts": 1, "effects": 1})
        self.assertEqual(detail["last_job_from_enqueue_commit_us"]["settled_us"], 25_000_000)
        self.assertEqual(detail["penultimate_settlement_from_enqueue_commit_us"], 2500)
        self.assertEqual(detail["jobs_at_last_settlement"], 1)

    def test_overlap_and_batched_terminal_bound_use_the_latest_counterfactual_job(self):
        example = evidence("short", "mesh")
        first, second, third = example["jobs"][:3]
        commit = example["transactions"][0]["committed_us"]
        first["settled_us"] = third["settled_us"] = commit + 8000
        third["settled_xid"] = first["settled_xid"]
        second["effect_committed_us"], second["settled_us"] = commit + 6800, commit + 7000
        result = metrics(example)
        self.assertEqual(result["largest_terminal_batch"], 2)
        self.assertEqual(result["window_us"]["commit"], 8000)
        self.assertEqual(result["fixed_timeline_mesh_bound_saved_us"]["second_to_terminal"], 1200)
        self.assertGreater(result["residence_us"]["claim_to_terminal"], result["window_us"]["commit"])
        self.assertEqual(result["middle80_ranks"], [12, 116])
        self.assertEqual(result["middle80_completion_intervals"], 104)
        self.assertEqual(result["window_us"]["middle80"], 0)
        self.assertIsNone(result["jobs_per_s"]["middle80"])
        third["settled_us"] -= 1
        with self.assertRaisesRegex(ValueError, "multiple commit timestamps"):
            metrics(example)

    def test_claim_coverage_quantization_and_sparse_arrivals(self):
        example = evidence("sparse", "rust")
        sparse = metrics(example)
        self.assertNotIn("claim_to_entry", sparse["segments"])
        self.assertNotIn("claim_to_terminal", sparse["residence_us"])
        self.assertTrue(sparse["residence_us"]["entry_to_terminal"])
        self.assertEqual(sum(sparse["sparse"].values()), sparse["window_us"]["submit"])
        self.assertFalse(sparse["fixed_timeline_mesh_bound_saved_us"])
        example["jobs"][0]["claimed_us"] = example["jobs"][0]["entered_us"]
        with self.assertRaisesRegex(ValueError, "unexpected claim coverage"):
            metrics(example)

        example = evidence("short", "mesh")
        example["transactions"][0]["committed_us"] += 1
        observed = metrics(example)
        self.assertEqual(observed["segments"]["commit_to_claim"]["reversed"], len(example["jobs"]))
        self.assertEqual(observed["segments"]["commit_to_claim"]["mean_ms"], -0.001)
        self.assertFalse(observed["fixed_timeline_mesh_bound_saved_us"])
        del example["jobs"][0]["claimed_us"]
        with self.assertRaisesRegex(ValueError, "unexpected claim coverage"):
            metrics(example)

    def test_ratios_pair_within_group_and_keep_commit_and_submit_separate(self):
        by_group = []
        for group, (mesh_commit, peer_commit, mesh_submit, peer_submit) in enumerate(
                ((10, 20, 40, 30), (20, 80, 90, 90), (30, 60, 130, 150)), 1):
            def arm(commit, submit, summed):
                return {"window_us": {"commit": commit, "submit": submit, "middle80": 0},
                        "first_claim_to_last_terminal_ms": 1,
                        "jobs_per_s": {"commit": 1 / commit, "submit": 1 / submit, "middle80": None},
                        "residence_us": {"entry_to_terminal": summed}, "segments": {},
                        "fixed_timeline_mesh_bound_saved_us": {}}
            mesh = arm(mesh_commit, mesh_submit, 100)
            peer = arm(peer_commit, peer_submit, 50)
            by_group.append({"group": group, "order": list(stages.ENGINES),
                             "arms": {"mesh": mesh, **dict.fromkeys(stages.ENGINES[1:], peer)}})
        pair = stages.summarize_profile(by_group)["paired"]["oban"]
        self.assertEqual(pair["windows"]["commit"]["mesh_to_peer"], {"n": 3, "median": 2, "min": 2, "max": 4})
        self.assertEqual(pair["windows"]["commit"]["mesh_wins"], 3)
        self.assertEqual(pair["windows"]["submit"]["mesh_wins"], 1)
        self.assertEqual(pair["windows"]["submit"]["ties"], 1)
        self.assertEqual(pair["windows"]["middle80"]["undefined_pairs"], 3)
        self.assertEqual(pair["residence_vs_wall_time"]["commit_window"]["entry_to_terminal"]["opposite"], 3)
        self.assertEqual(pair["residence_vs_wall_time"]["post_claim_drain"]["entry_to_terminal"]["tied"], 3)


class ReplayTests(unittest.TestCase):
    def setUp(self):
        stages.TARGET.mkdir(parents=True, exist_ok=True)
        self.directory = Path(self.enterContext(TemporaryDirectory(dir=stages.TARGET)))
        self.enterContext(patch.object(matrix, "RESULTS", self.directory))
        self.config = deepcopy(CONFIG)
        self.config["engines"]["go"]["verified_files"] = {"/bench/go": "b" * 64}
        self.args = Namespace(group=None, profile=["short", "noop"])

    def initialize(self, name, kind, config=None, controls=None):
        path = self.directory / name
        path.mkdir()
        series = matrix.initialize(path, kind, config or self.config, controls=controls)
        return path, series

    def test_stable_control_replay_keeps_incomplete_warmup_and_mixed_group_out(self):
        path, series = self.initialize("controls", "controls")
        short = [b for b in series["plan"] if b["profile"] == "short"]
        noop = series["plan"][0]
        first = create_block(path, short[0], self.config, 1)
        second = create_block(path, short[1], self.config, 2)
        incomplete = create_block(path, noop, self.config, 3)
        (incomplete / "warmup-go/raw.tar.gz").unlink()
        # A copy of group 1 under a different group fails the planned order check.
        shutil.copytree(first, path / "blocks" / "02-noop.01")
        report, issues = stages.analyze_series(path.name, self.args)
        self.assertEqual(report["scoring_phase"], "control")
        self.assertEqual(report["missing_blocks"], ["01-noop", "02-noop"])
        self.assertEqual(set(issues), {"01-noop.01", "02-noop.01"})
        self.assertEqual([b["group"] for b in report["blocks"]], [1, 2])
        self.assertEqual(report["blocks"][0]["order"], report["blocks"][1]["order"][::-1])
        self.assertEqual(stages.json_bytes(report), stages.json_bytes(stages.analyze_series(path.name, self.args)[0]))
        report["replay"] = "replay"
        rendered = stages.report_markdown(report)
        self.assertIn("| Recorded queue claim → handler entry mean (ms) | 0.100 | 0.100 | 0.100 | — |", rendered)

        # Copy both request and archive so the existing request/archive check
        # still passes. The grafted arm runs outside the new block's serial order.
        for filename in ("request.json", "raw.tar.gz"):
            shutil.copyfile(first / "control-mesh" / filename, second / "control-mesh" / filename)
        with self.assertRaisesRegex(ValueError, "violate serial matrix order"):
            stages.analyze_series(path.name, self.args)

    def test_baseline_uses_complete_controls_only_as_authority(self):
        control, rotation = self.initialize("controls", "controls")
        for position, block in enumerate(rotation["plan"], 1):
            create_block(control, block, self.config, position)
        authority = matrix.verify_controls(control.name, self.config)
        baseline, series = self.initialize("baseline", "baseline", controls=authority)
        short = next(b for b in series["plan"] if b["profile"] == "short")
        create_block(baseline, short, self.config, 12)

        report, _ = stages.analyze_series(baseline.name, self.args)
        self.assertEqual(report["scoring_phase"], "measured")
        self.assertEqual([(b["profile"], b["group"]) for b in report["blocks"]], [("short", 1)])
        self.assertEqual(report["profiles"]["short"]["paired"]["oban"]["windows"]["commit"]["mesh_to_peer"]["n"], 1)
        self.assertIn(str((control / "series.json").relative_to(stages.ROOT)), report["sources"])
        short_next = next(b for b in series["plan"] if b["profile"] == "short" and b["group"] == 2)
        create_block(baseline, short_next, self.config, 13, wrong_adapter_phase=("measured", "rust"))
        with self.assertRaisesRegex(ValueError, "adapter runtime phase"):
            stages.analyze_series(baseline.name, self.args)

    def test_diagnostic_and_original_river_are_ineligible(self):
        diagnostic, _ = self.initialize("setup-diagnostic", "diagnostic")
        with self.assertRaisesRegex(ValueError, "only measured or rotation-control"):
            stages.analyze_series(diagnostic.name, self.args)
        self.config["engines"]["go"]["artifact_sha256"] = matrix.RIVER_PREDECODED_SHA256
        controls, _ = self.initialize("predecoded", "controls")
        with self.assertRaisesRegex(ValueError, "replacement River"):
            stages.analyze_series(controls.name, self.args)


if __name__ == "__main__":
    unittest.main()
