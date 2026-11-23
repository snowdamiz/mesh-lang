#!/usr/bin/env python3
"""Reconcile ordinary and comparison profiles from existing raw evidence."""

import argparse
from collections import Counter
import importlib.util
import json
from pathlib import Path
import sys
import tarfile

from assemble import LANE, NATIVE, ROOT, digest
from freeze import verify

sys.path.insert(0, str(ROOT / "tools/durable-jobs-bench"))
import run
from verify import reconcile

CONTRACT = "ordinary-trusted-shared-process"
ROWS = 64
PAYLOAD = 512 * 1024


def reads_local(root):
    return lambda name: run.read_json(root / name)


def reads_archive(archive):
    return lambda name: run.file_json(archive, "adapter/" + name)


def slots(mask):
    return {int(pid): tuple(cpus) for pid, cpus in mask.items()}


def ordinary(read, score, policy, pins, cpus):
    ledger, cert, storage = (read(name) for name in ("ledger.json", "assembly.json", "payload-storage.json"))
    external = policy == "external"
    if (ledger["execution_contract"] != CONTRACT or cert["execution_contract"] != CONTRACT
            or cert["storage_policy"] != policy or cert["producer"] != "mesh-direct-file"
            or cert["runner_sha256"] != pins["runner_sha256"] or cert["broker_sha256"] != pins["broker_sha256"]):
        raise ValueError("producer or execution contract differs from the sealed lane")
    running = set(cert["runner_pids"])
    if (len(running) != ledger["runners"] or cert["broker_pid"] in running
            or not {job["worker_pid"] for job in ledger["jobs"]} <= running):
        raise ValueError("handlers did not execute in the retained ordinary runner processes")
    if cpus:
        masks = slots(cert["affinity"])
        postgres_and_runners = slots(read("process-affinity.json"))
        if (len(postgres_and_runners) != len(running) + 1 or len(masks) != len(running) + 1
                or set(masks) != running | {cert["broker_pid"]}
                or not running <= postgres_and_runners.keys()
                or any(mask != tuple(cpus) for mask in (*masks.values(), *postgres_and_runners.values()))):
            raise ValueError("runner, broker, or postgres CPU affinity differs")
        launches = {int(pid): evidence for pid, evidence in cert["launches"].items()}
        expected = {pid: [str(Path(pins["runner_artifacts"]) / "mesh"), "run"] for pid in running}
        expected[cert["broker_pid"]] = [pins["broker_path"], "session"]
        adapter_pid = cert["adapter_pid"]
        if launches.keys() != expected.keys() or adapter_pid in launches or adapter_pid <= 0:
            raise ValueError("incomplete owned launch evidence")
        for pid, record in launches.items():
            method = record["method"]
            if (record["argv"] != expected[pid] or record["parent_pid"] != adapter_pid
                    or type(record["start_ticks"]) is not int or record["start_ticks"] <= 0
                    or method not in {"owned-launch", "proc-exe"}
                    or record["observed_executable"] != (expected[pid][0] if method == "proc-exe" else None)):
                raise ValueError("live executable evidence differs from the pinned launch")
    tls = read("postgres-tls-sessions.json")
    if (not all(row["tls"] for row in tls)
            or Counter(row["role"] for row in tls) != {
                "compiled_caller": 32, "parity_subject": 32, "parity_producer": 1}
            or len({row["backend"] for row in tls}) != 65 or score["peak_connections"] != 65):
        raise ValueError("large V1 proof requires the frozen 65-session topology")
    writes = read("trusted-application-statements.json")
    for table in ("attempts", "effects"):
        records = [q for q in writes if q["query"].startswith("INSERT INTO public.mesh_jobs_parity_" + table)]
        if len(records) != 1 or records[0]["rows"] != ROWS or records[0]["calls"] != ROWS:
            raise ValueError("wrong direct application statement counts")
    setup, persisted = (storage[name] for name in ("setup", "persisted"))
    before, after = (setup[name] for name in ("before", "after"))
    tx, = ledger["transactions"]
    if (storage["engine"] != "mesh" or storage["policy"] != ("EXTERNAL" if external else "EXTENDED")
            or setup["rows_before"] != 0
            or after != dict(before, attstorage="e" if external else "x") or before["attstorage"] != "x"
            or after != persisted["metadata"] or not persisted["trigger_absent"]
            or (after["relation"], after["owner"], after["column"], after["type"], after["persistence"])
            != ("mesh_jobs.jobs", "parity_subject", "args", "jsonb", "p")
            or int(after["toast_oid"]) <= 0 or int(after["relation_oid"]) <= 0
            or setup["ddl"] != ("ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTERNAL" if external else None)
            or not 0 < setup["ddl_start_us"] <= setup["ddl_end_us"] < tx["submitted_us"]
            or not score["last_settlement_us"] <= storage["verify_start_us"] <= storage["verify_end_us"]):
        raise ValueError("payload storage changed outside the intended disposable setup")
    jobs = {job["id"]: job for job in ledger["jobs"]}
    settled = {job["settled_xid"] for job in jobs.values()}
    app_xids = {job[xid] for job in jobs.values() for xid in ("attempt_xid", "effect_xid")}
    if ROWS != len(jobs) or ROWS != len(settled) or 2 * ROWS != len(app_xids) or (
        settled & (app_xids | {str(int(tx["xid"]) % 2**32)})
    ):
        raise ValueError("expected independent attempt, effect, product, and terminal XIDs")
    stored = persisted["rows"]
    if len(stored) != ROWS or set(jobs) != {row["id"] for row in stored}:
        raise ValueError("payload snapshot differs from the native job population")
    for row in stored:
        job = jobs[row["id"]]
        if ((row["job_key"], row["payload_sha256"], row["settled_xid"])
                != (job["job_key"], job["payload_sha256"], job["settled_xid"])
                or row["payload_bytes"] != PAYLOAD or row["native_state"] != "completed"
                or row["stored_column_bytes"] <= 0
                or (external and (row["compression"] is not None or row["stored_column_bytes"] < PAYLOAD))):
            raise ValueError("persisted payload does not match the committed job")
    stored_bytes = sum(row["stored_column_bytes"] for row in stored)
    if external and persisted["toast_heap_bytes"] < stored_bytes:
        raise ValueError("TOAST heap is smaller than the external values")
    # A coarse observer can miss all short-lived executing rows. Use the
    # latest possible millisecond claim stamp for a conservative peak.
    edges = sorted((stamp, delta) for job in jobs.values() for stamp, delta in (
        (min(job["claimed_us"] + 999, job["entered_us"]), 1), (job["settled_us"], -1)))
    active = peak = 0
    for _, delta in edges:
        active += delta
        peak = max(peak, active)
    if not 1 <= peak <= 64:
        raise ValueError("claim/terminal ledger violates the frozen slot limit")
    return dict(launch_checks=dict(Counter(record["method"] for record in cert["launches"].values())),
                conservative_ledger_peak_slots=peak, payloads=ROWS, payload_bytes=ROWS * PAYLOAD, stored_column_bytes=stored_bytes,
                toast_heap_bytes=persisted["toast_heap_bytes"], terminal_transactions=len(settled),
                compression=dict(Counter(row["compression"] or "none" for row in stored)),
                application_commits=len(app_xids))


def summarize(score, size):
    return dict(completed=score["completed"], submit_jobs_per_s=score["submit_jobs_per_s"],
                commit_jobs_per_s=score["commit_jobs_per_s"],
                submit_to_commit_ms=score["first_submit_to_commit_ms"],
                submit_to_terminal_ms=(score["last_settlement_us"] - score["first_submit_us"]) / 1000,
                commit_to_terminal_ms=(score["last_settlement_us"] - score["first_commit_us"]) / 1000,
                peak_connections=score["peak_connections"], sampled_peak_slots=score["peak_executing"],
                **size)


def native(root, policy, config, phase, label="ordinary"):
    lane = config["lanes"][label]
    engine, inventory = (lane[key] for key in ("engine", "inventory"))
    score = run.score_arm(root, {"profile": "large"}, phase, engine, inventory)
    host = (score["host_boot_id"], score["host_kernel"], score["pg_mount"]["maj:min"])
    if list(host) != config["host"]:
        raise ValueError(f"{label}: different native host or disk")
    raw = root / "raw.tar.gz"
    with tarfile.open(raw, "r:gz") as archive:
        read = reads_archive(archive)
        if engine in {"mesh", "oban"}:
            affinity = slots(read("process-affinity.json"))
            if len(affinity) != 9 or any(mask != tuple(config["cpus"]) for mask in affinity.values()):
                raise ValueError(f"{label}: PostgreSQL or runner CPU affinity differs")
        if label == "ordinary":
            pins = run.read_json(LANE / "source-retry" / NATIVE / "assembly-pins.json")
            size = ordinary(read, score, policy, pins, config["cpus"])
        elif policy == "external":
            if engine == "mesh":
                first, rows, ledger = (read(name) for name in ("payload-storage-before.json",
                    "payload-storage-after.json", "ledger.json"))
                if (first["before"]["attstorage"], first["after"]["attstorage"]) != ("x", "e") or (
                    first["before"]["rows"] != 0 or first["after"]["rows"] != 0
                    or len(rows) != ROWS or {(r["id"], r["payload_sha256"]) for r in rows}
                    != {(j["id"], j["payload_sha256"]) for j in ledger["jobs"]}
                    or any(row["compression"] is not None or row["state"] != "completed" or row["attempt"] != 1 for row in rows)
                ):
                    raise ValueError("protected EXTERNAL control failed its storage readback")
                size = {"payload_bytes": ROWS * PAYLOAD, "stored_column_bytes": sum(r["stored_bytes"] for r in rows)}
            else:
                size = peer_storage().check(read("payload-storage.json"), read("ledger.json"), engine)
        else:
            size = {}  # Existing default control slots lack per-column storage receipts.
    return dict(raw_sha256=digest(raw), **summarize(score, size))


def peer_storage():
    # The peer proof and its storage module are the upstream versions frozen in inputs.
    for name, filename in (("storage", "storage-upstream.py"), ("peer_replay", "peer-storage-replay.py")):
        spec = importlib.util.spec_from_file_location(name, LANE / "inputs" / filename)
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
    return module


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("local", "native", "block"))
    parser.add_argument("policy", choices=("external", "default"))
    parser.add_argument("evidence", type=Path)
    args = parser.parse_args()
    if args.mode == "local":
        read = reads_local(args.evidence)
        ledger = read("ledger.json")
        if ledger["local_smoke"] is not True:
            raise ValueError("expected a local diagnostic")
        pins = run.read_json(LANE / "inputs/local-pins.json")
        artifact = {"version": read("build.json")["version"], "artifact_sha256": pins["runner_manifest_sha256"]}
        score = reconcile(ledger, "mesh", "large", artifact)
        result = summarize(score, ordinary(read, score, args.policy, pins, []))
    else:
        freeze = LANE / "freeze-retry"
        verify(freeze)
        config = run.read_json(freeze / f"{args.policy}.json")
        if args.mode == "native":
            result = native(args.evidence, args.policy, config, "diagnostic")
        else:
            block = run.read_json(args.evidence / "order.json")
            if block["profile"] != "large" or set(config["lanes"]) != set(block["order"]):
                raise ValueError("block differs from the frozen large comparison")
            result = {phase: {
                label: native(args.evidence / f"{phase}-{label}" / f"{phase}-{config['lanes'][label]['engine']}",
                              args.policy, config, phase, label)
                for label in block["order"]
            } for phase in block["phases"]}
    print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))


if __name__ == "__main__":
    main()
