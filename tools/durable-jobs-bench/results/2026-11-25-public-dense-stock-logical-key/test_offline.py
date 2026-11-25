"""Replay the six retained 8×8 stock arms and check prospective gate edge cases."""

from copy import deepcopy
import json
import runpy
import sys
import tarfile
import unittest

import controller as c
from score import grant_plans, pilot_gate, plan_gate, workload

HISTORY = c.TRACE / "evidence/native"


def history():
    recorded = c.run.read_json(HISTORY/"final-replay.json")["groups"]
    current = c.config()
    original = runpy.run_path(str(c.TRACE/"controller.py"))["inputs"]()
    cohort = []
    for path in sorted((HISTORY/"blocks").glob("[01][12]-noop.*")):
        trial = c.run.read_json(path/"order.json")
        for lane, arm in recorded[str(trial["group"])][1]["arms"].items():
            if not lane.startswith("stock-"):
                continue
            folder = path/f"measured-{lane}/measured-mesh"
            archive_path = folder/"raw.tar.gz"
            inventory = original["lanes"][lane]["inventory"]
            base = c.run.score_arm(folder, trial, "measured", "mesh", inventory)
            assert [base["host_boot_id"], base["host_kernel"], base["pg_mount"]["maj:min"]] == current["host"]
            assert c.run.digest(archive_path) == arm["raw_sha256"]
            with tarfile.open(archive_path) as archive:
                stock = workload(archive)
            assert stock["claim_calls"] == arm["observer"].get("raw_claim_calls", stock["claim_calls"])
            for key in ("first_submit_us", "first_commit_us", "last_settlement_us"):
                assert stock[key] == arm[key] == base[key]
            stock["path"] = str(archive_path.relative_to(c.ROOT))
            stock["sha256"] = arm["raw_sha256"]
            lock = [row for row in arm["elapsed_sql"]["submit_to_observer"]
                    if "FROM mesh_jobs.queues" in (row["query"] or "") and "FOR NO KEY UPDATE" in row["query"]
                    and "*        FROM" in row["query"]]
            if lane == "stock-trace":
                row, = lock
                assert row["calls"] == stock["claim_calls"]
                stock["queue_lock_stmt_elapsed_ms"] = row["total_exec_time"]
                jobs, = [row for row in arm["relations_off_clock"] if row["relname"] == "jobs"]
                stock["whole_cluster_jobs_index_fetches"] = jobs["idx_tup_fetch"]
                stock["acquired_claim_wait_lower_bound_ms"] = sum(
                    row["elapsed_ms"] for row in arm["diagnostic_log"]["acquired_waits"])
            cohort.append(dict(group=trial["group"], lane=lane, **stock))
    assert len(cohort) == 6
    return cohort


class Offline(unittest.TestCase):
    def test_historical_stock(self):
        self.assertEqual(history(), c.run.read_json(c.HERE/"history.json"))

    def test_cpu_bracket_and_reused_pid(self):
        frame = dict(postmaster=1, clock_ticks=100, before=dict(begin_us=90, end_us=95,
                     rows={"1": [0, 10, 3], "2": [1, 11, 7]}),
                     after=dict(begin_us=110, end_us=115, rows={"1": [0, 10, 4], "2": [1, 11, 9]}))
        clocks = dict(first_submit_us=100, last_settlement_us=105)
        self.assertEqual(c.pg_cpu(frame, clocks, [2]),
                         dict(ms=30, stable_pids=2, unmatched=[], slack_us=20))
        frame["after"]["rows"]["2"][1] = 12
        with self.assertRaises(AssertionError):
            c.pg_cpu(frame, clocks, [2])

    def test_peer_gate_requires_every_resource_and_both_rotations(self):
        metrics = ("submit_us", "drain_us", "effect_from_commit_us", "runner_cpu_ms", "pg_cpu_ms", "host_cpu_ms")
        groups = {0: dict(plan_gate=dict(passed=True))}
        for group in (1, 2):
            groups[group] = dict(arms={lane: dict.fromkeys(metrics, value) for lane, value in
                                       (("stock-a", 1000), ("stock-b", 1002), ("key", 940))})
        self.assertTrue(pilot_gate(groups)["passed"])
        for metric in metrics:
            altered = deepcopy(groups)
            altered[2]["arms"]["key"][metric] = 950
            self.assertFalse(pilot_gate(altered)["passed"])
        groups[2]["arms"]["stock-b"]["drain_us"] = 1060
        self.assertFalse(pilot_gate(groups)["passed"])
        self.assertFalse(pilot_gate({0: groups[0], 1: groups[1]})["passed"])

    def test_node_attribution_from_retained_grant_plans(self):
        from plans import reduce
        for lane in c.DIAGNOSTIC:
            native = c.KEY/"evidence/native/blocks"
            path, = native.glob(f"00-noop.*/measured-{lane}/measured-mesh/raw.tar.gz")
            with tarfile.open(path) as archive:
                ledger = c.member(archive, "ledger")
                clocks = dict(first_submit_us=ledger["transactions"][0]["submitted_us"],
                              first_commit_us=ledger["transactions"][0]["committed_us"],
                              last_settlement_us=max(j["settled_us"] for j in ledger["jobs"]))
                obs = reduce(*(c.member(archive, member) for member in
                               ("grant-plans", "grant-frames", "grant-closed")), clocks)
            # This published 2×32 fixture drives the attribution reducer only.
            # The 8×8 runtime validates a width of at most eight.
            obs["samples"] = [sample for sample in obs["samples"]
                              if sample["nodes"][0]["emitted_approx"] <= 8]
            result = grant_plans(obs, lane == "plan-key")
            self.assertGreater(result["totals"]["jobs"], 0)
            if lane == "plan-key":
                self.assertEqual(result["totals"]["broad"], 0)
            else:
                self.assertGreater(result["totals"]["target_visits"], result["totals"]["pick_visits"])
            self.assertFalse(plan_gate(result, result)["passed"])


if __name__ == "__main__":
    if sys.argv[1:] == ["history"]:
        print(json.dumps(history(), indent=2, sort_keys=True))
    else:
        unittest.main()
