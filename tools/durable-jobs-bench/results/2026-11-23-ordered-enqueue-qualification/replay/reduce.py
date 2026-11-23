"""Independently replay archived producer and native worker ledgers."""
import argparse
import json
from pathlib import Path
from statistics import median
import sys
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = next(path for path in HERE.parents if (path / "tools/durable-jobs-bench/run.py").is_file())
SUITE = ROOT / "tools/durable-jobs-bench"
sys.path.insert(0, str(SUITE))
from run import check_inventory, digest
from verify import PROFILES, expected_inputs, reconcile

BASE = "/var/lib/durable-jobs-bench/native"
CANDIDATE = "77c447f773a4d0af5c5578873a2410301b3ba069"
CONTROL = "3f69b150820fd01ba716d533be4f2eedea25ed27"
ARMS = ("stock-a", "stock-b", "candidate")


def read(path):
    return json.loads(path.read_bytes())


def read_entry(archive, name):
    return json.load(archive.extractfile("./" + name))


def span(samples):
    return dict(min=min(samples), median=median(samples), max=max(samples))


def compile_inventory(archive, out, inventory):
    name = archive.extractfile("./paths.txt").read().decode().splitlines()
    source, package = name
    assert source.startswith(f"{BASE}/source-ordered-enqueue-v")
    assert package == f"{BASE}/builds/mesh-ordered-enqueue-v{source.rsplit('-v', 1)[1]}"
    candidate_bytes = archive.extractfile("./mesh.json").read()
    manifest = json.loads(candidate_bytes)
    assert (manifest["system"], manifest["architecture"], manifest["engine"], manifest["version"]) == (
        "Linux", "x86_64", "mesh", CANDIDATE)
    stock = read_entry(archive, "stock-mesh.json")
    assert stock["version"] == CONTROL and stock["fixture"] == manifest["fixture"] == "linux-warm-replay-20261122"
    assert stock["files"].keys() == manifest["files"].keys()
    assert stock["files"]["mesh"] != manifest["files"]["mesh"]
    changed = {n for n, checksum in stock["files"].items() if checksum != manifest["files"][n]}
    assert changed == {"mesh", "mesh-toolchain.txt"}, changed
    mesh = inventory["engines"]["mesh"]
    stock_argv = ["/usr/bin/env", f"NATIVE_JOBS_ARTIFACTS={BASE}/builds/mesh-v1",
                  f"{BASE}/source-v5/tools/durable-jobs-bench/native/run", "mesh"]
    assert mesh["argv"] == stock_argv and mesh["version"] == CONTROL
    candidate_path = out / "mesh.json"
    candidate_path.write_bytes(candidate_bytes)
    manifest_sha = digest(candidate_path)
    inventory["engines"]["mesh"] = {
        **mesh, "version": CANDIDATE, "artifact_sha256": manifest_sha,
        "argv": [stock_argv[0], f"NATIVE_JOBS_ARTIFACTS={package}",
                 f"{source}/tools/durable-jobs-bench/native/run", "mesh"],
        "verified_files": {
            filename.replace(f"{BASE}/source-v5/", f"{source}/", 1): checksum
            for filename, checksum in mesh["verified_files"].items()
        },
    }
    check_inventory(inventory)
    return {"manifest_sha256": manifest_sha, "candidate": manifest, "stock": stock}


def verify_semantic(archive, step, reference):
    row = read_entry(archive, "rows/row.json")
    ledger = read_entry(archive, "rows/ledger.json")
    arm, profile = step.removeprefix("semantic-").split("-")
    assert row["profile"] == profile and ledger["profile"] == profile
    if profile == "dense":
        profile = "noop"
        ledger["profile"] = profile
    assert row["phase"] == ledger["phase"] == "diagnostic" and row["local_smoke"] is False
    result = reconcile(ledger, "mesh", profile, dict(phase="diagnostic", local_smoke=False,
                       version=reference[arm]["version"], artifact_sha256=reference[arm]["artifact_sha256"]))
    masks = read_entry(archive, "rows/process-affinity.json")
    assert len(masks) == PROFILES[profile][0] + 1 and all(mask == [0, 1] for mask in masks.values())
    records = read_entry(archive, "rows/application-records.json")
    assert {key: len(values) for key, values in records.items()} == {
        key: result["completed"] for key in ("attempts", "effects")
    }
    assert all(tx["submitted_us"] <= tx["committed_us"] <= tx["acknowledged_us"] for tx in ledger["transactions"])
    return {"profile": profile, "jobs": result["completed"], "work_records": sum(map(len, records.values())),
            "transactions": len(ledger["transactions"]), "artifact_sha256": row["artifact_sha256"]}


def verify_producer(archive, stage, build):
    wire = stage.startswith("producer-wire-")
    shape = stage.removeprefix("producer-").removeprefix("wire-")
    profile = "noop" if shape in ("component", "dense") else shape
    expected = expected_inputs(profile)
    if shape == "component":
        request, = expected
        expected = {request: dict(list(expected[request].items())[:128])}
    expected_count = sum(len(group) for group in expected.values())
    assert read_entry(archive, "rows/inputs.json")["digest"] == PROFILES[profile][3]
    series = read_entry(archive, "rows/series.json")
    assert len(series) == (3 if wire else 18)
    result = {arm: {} for arm in ARMS}
    for index, run in enumerate(series, 1):
        group, arm = run["group"], run["arm"]
        assert group not in result[arm] and group == (index + 2) // 3
        assert run["executable_sha256"] == build["candidate" if arm == "candidate" else "stock"]["files"]["mesh"]
        prefix = f"rows/{index:02}-{group:02}-{arm}/"
        ledger = read_entry(archive, prefix + "durable-ledger.json")
        assert ledger["counts"] == [expected_count, expected_count, len(expected), 0, 0]
        assert ledger["commits"].keys() == expected.keys()
        pg = read_entry(archive, prefix + "postgres-session.json")
        assert pg["user"] == "parity_producer" and pg["tls"] is True and pg["version"] == "170011"
        assert pg["durability"] == ["on"] * 4
        before, after = (read_entry(archive, prefix + "process-" + edge + ".json") for edge in ("before", "after"))
        ticks = {}
        for label in ("producer", "backend"):
            a, b = before[label], after[label]
            assert (a["pid"], a["start_ticks"], a["affinity"]) == (b["pid"], b["start_ticks"], b["affinity"])
            assert a["affinity"] == [0, 1]
            ticks[label] = {stat: b[stat] - a[stat] for stat in ("user_ticks", "kernel_ticks")}
            assert min(ticks[label].values()) >= 0
        host = [read_entry(archive, prefix + "host-" + edge + ".json") for edge in ("before", "after")]
        assert all(snapshot["affinity"] == [0, 1] for snapshot in host)
        loads = [snapshot["loadavg"][0] for snapshot in host]
        cpu = [{parts[0]: list(map(int, parts[1:])) for line in snapshot["cpu"].splitlines()
                if (parts := line.split()) and parts[0] in ("cpu0", "cpu1")} for snapshot in host]
        deltas = [sum(cpu[1][name][field] - cpu[0][name][field] for name in ("cpu0", "cpu1")) for field in range(8)]
        assert min(deltas) >= 0
        steal_pct = 100 * deltas[7] / max(1, sum(deltas))
        sql = read_entry(archive, prefix + "pg-stat-statements.json")
        statement, = [data for query, data in sql.items() if query.startswith("SELECT mesh_jobs.enqueue(")]
        assert statement["calls"] == expected_count
        offset = 0
        producer_xids, ids = set(), set()
        for seq, (request, expected_jobs) in enumerate(expected.items(), 1):
            raw = read_entry(archive, prefix + f"response-{seq:03}.json")
            xid, stamp, engine, ns = ledger["commits"][request]
            assert engine == "mesh" and int(xid) > 2 and xid not in producer_xids
            producer_xids.add(xid)
            receipt = raw["response"]
            assert receipt["xid"] == xid and receipt["count"] == len(expected_jobs)
            rows = ledger["jobs"][offset:offset + len(expected_jobs)]
            assert list(expected_jobs) == [row["key"] for row in rows]
            assert receipt["ids"] == {row["key"]: row["id"] for row in rows}
            for row in rows:
                job = row["id"]
                assert job not in ids
                ids.add(job)
                assert row["namespace"] == ns and row["job_xid"] == row["receipt_xid"] and int(row["job_xid"]) > 2
                assert row["job_commit_us"] == row["receipt_commit_us"] == stamp
                assert (row["payload_sha256"], row["hold_ms"], row["fail_first"]) == expected_jobs[row["key"]]
                assert row["receipt"] == {"id": job, "effect_key": f"{job}/1", "state": "available",
                                          "conflict": False, "inserted": True}
            offset += len(rows)
            sample = run["requests"][seq - 1]
            assert raw["submitted_us"] <= stamp <= raw["acknowledged_us"]
            assert sample["commit_ms"] == (stamp - raw["submitted_us"]) / 1000
            assert sample["ack_ms"] == raw["ack_ms"] and sample["tls"] == raw["tls"]
            if shape == "component":
                assert receipt["receipts"] == [row["receipt"] for row in rows]
        ordinals = [row["ordinal"] for row in ledger["jobs"]]
        assert len(ids) == expected_count and ordinals == sorted(set(ordinals))
        assert run["commit_ms"] == median(r["commit_ms"] for r in run["requests"])
        assert run["ack_ms"] == median(r["ack_ms"] for r in run["requests"])
        tls = {key: sum(r["tls"][key] for r in run["requests"])
               for key in ("client_tls_records", "client_bytes", "server_tls_records", "server_bytes")} if wire else None
        result[arm][group] = {"commit_ms": run["commit_ms"], "ack_ms": run["ack_ms"], "statement_ms": statement["execute_ms"],
                              "loadavg_1m": loads, "process_ticks": ticks, "cpu_steal_percent": steal_pct, "tls": tls}
    if wire:
        counters = {arm: result[arm][1]["tls"] for arm in ARMS}
        assert counters["stock-a"] == counters["stock-b"]
        if shape in ("large", "sparse"):
            assert counters["candidate"] == counters["stock-a"]
        else:
            assert counters["candidate"]["client_tls_records"] < counters["stock-a"]["client_tls_records"]
            assert counters["candidate"]["server_tls_records"] == counters["stock-a"]["server_tls_records"]
        return counters
    return {
        "groups": len(result["candidate"]), "runs": result,
        "distributions": {arm: {metric: span([run[metric] for run in result[arm].values()])
                                for metric in ("commit_ms", "ack_ms", "statement_ms", "cpu_steal_percent")} for arm in ARMS},
        "candidate_minus_stock_ms": {
            arm: {metric: [result["candidate"][group][metric] - run[metric] for group, run in result[arm].items()]
                  for metric in ("commit_ms", "ack_ms")} for arm in ARMS[:2]
        },
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("receipts", type=Path)
    parser.add_argument("out", type=Path)
    args = parser.parse_args()
    args.out.mkdir(exist_ok=False)
    attempts = {}
    successes = {}
    for request_path in sorted(args.receipts.rglob("request.json")):
        request = read(request_path)
        step = request["stage"]
        raw = request_path.parent / "raw.tar.gz"
        record = {"receipt": request["receipt"], "step": step, "grant": request["grant"], "build": request["build"]}
        if raw.is_file():
            with tarfile.open(raw) as archive:
                code = int(archive.extractfile("./exit.txt").read())
            record.update(archive_sha256=digest(raw), exit=code)
            if code == 0:
                successes[(request["build"], step)] = raw
        attempts[record["receipt"]] = record
    builds = [idx for idx, step in successes if step == "build"]
    out = {"attempts": attempts}
    if builds:
        selected = max(builds, key=lambda name: int(name[1:]))
        inventory_path = SUITE / "results/2026-11-23-baseline/inventory.json"
        inventory = read(inventory_path)
        stock = inventory["engines"]["mesh"]
        with tarfile.open(successes[selected, "build"]) as archive:
            build = compile_inventory(archive, args.out, inventory)
        reference = {"stock": stock, "candidate": inventory["engines"]["mesh"]}
        out.update(build=selected, executable_sha256=build["candidate"]["files"]["mesh"],
                   manifest_sha256=build["manifest_sha256"], semantic={}, producer={}, wire={}, fault={})
        for (number, step), raw in successes.items():
            if number != selected or step == "build":
                continue
            with tarfile.open(raw) as archive:
                if step.startswith("semantic-"):
                    out["semantic"][step] = verify_semantic(archive, step, reference)
                elif step == "producer-faults":
                    for index, fault in enumerate(("sql-error", "disconnect"), 1):
                        row = read_entry(archive, f"rows/{index:02}-01-{fault}/fault.json")
                        assert row == dict(attempts=4, jobs=0, receipts=0, product=0, exit=1)
                        out["fault"][fault] = row
                    rollback = read_entry(archive, "rows/03-01-candidate/explicit-rollback.json")
                    assert rollback["recovered_same_session"] is True and rollback["commit_timestamp"] is None
                    out["fault"]["explicit-rollback"] = rollback
                elif step.startswith("producer-wire-"):
                    out["wire"][step.removeprefix("producer-wire-")] = verify_producer(archive, step, build)
                elif step.startswith("producer-"):
                    out["producer"][step.removeprefix("producer-")] = verify_producer(archive, step, build)
                elif step == "wire-test":
                    log = archive.extractfile("./step.log").read()
                    assert b"ordered_enqueue_reads_a_backpressured_window_and_retires_a_truncated_connection ... ok" in log
                    out["fault"]["native-truncated-response-test"] = True
        assert all(inventory["engines"][peer] == spec for peer, spec in read(inventory_path)["engines"].items()
                   if peer != "mesh")
        (args.out / "inventory.json").write_text(json.dumps(inventory, indent=2, sort_keys=True) + "\n")
    (args.out / "replay.json").write_text(json.dumps(out, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
