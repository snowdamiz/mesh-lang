"""Verify the two archived native Apalis SETUP arms offline."""

from collections import Counter
from datetime import datetime, timezone
from hashlib import sha256
import importlib.util
import json
from pathlib import Path, PurePosixPath
import tarfile

HERE = Path(__file__).resolve().parent
SUITE = HERE.parents[1]
ROOT = SUITE.parents[1]


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


adapter = load_module("apalis_adapter", SUITE / "rust/run.py")
from verify import database_check, reconcile, require, xid


def digest(data):
    return sha256(data).hexdigest()


def manifest(data):
    entries = [line.split("  ", 1) for line in data.decode().splitlines()]
    sums = {name: checksum for checksum, name in entries}
    require(len(sums) == len(entries), "manifest repeats a path")
    return sums


def read_archive(name):
    filename = name + ".tar.gz"
    path = HERE / filename
    require(digest(path.read_bytes()) == manifest((HERE / "SHA256SUMS").read_bytes())[filename],
            f"{filename}: archive digest mismatch")
    with tarfile.open(path, "r:gz") as archive:
        files = {}
        for member in archive:
            relative = PurePosixPath(member.name)
            require(member.isfile() and not relative.is_absolute() and ".." not in relative.parts
                    and member.name not in files, f"{filename}: invalid member {member.name}")
            files[member.name] = archive.extractfile(member).read()
    expected = manifest(files["SHA256SUMS"])
    require({path: digest(data) for path, data in files.items() if path != "SHA256SUMS"} == expected,
            f"{filename}: archived member digests differ")
    return files


def verify_rows(jobs, transactions, writes, pids):
    receipt_ids = [(identity, key) for tx in transactions for key, identity in tx["enqueued_ids"].items()]
    require(Counter((row["id"], row["job_key"]) for row in jobs) == Counter(receipt_ids)
            and len({identity for identity, _ in receipt_ids}) == len(receipt_ids)
            and len({key for _, key in receipt_ids}) == len(receipt_ids),
            "native job rows and enqueue receipt identities differ")
    adapter.check_writes(jobs, transactions, writes, pids)
    require(all(row["job_type"] == adapter.QUEUE for row in jobs), "unexpected Apalis queue")

    # Index only after check_writes has verified the exact multiplicity of every row.
    attempts = {(row["job_id"], row["attempt"]): row for row in writes["attempts"]}
    effects = {row["job_id"]: row for row in writes["effects"]}
    application_xids = {xid(row["xid"], "application xid") for row in [*attempts.values(), *effects.values()]}
    producer_xids = {xid(tx["xid"], "producer xid") for tx in transactions}
    require(len(application_xids) == len(attempts) + len(effects)
            and producer_xids.isdisjoint(application_xids), "reused application transaction XID")
    for job in jobs:
        identity = job["id"]
        first, effect = attempts[identity, 1], effects[identity]
        require((first["xid"], first["committed_us"], effect["xid"], effect["committed_us"])
                == (job["attempt_xid"], job["attempt_committed_us"],
                    job["effect_xid"], job["effect_committed_us"]),
                "raw application commits differ from the job export")
        require(int(first["xid"]) < int(effect["xid"])
                and first["committed_us"] < effect["committed_us"] <= job["settled_us"],
                "raw application commits are out of order")
        require(xid(job["settled_xid"], "settled xid") not in application_xids,
                "terminal state shares an application transaction")
        if job["attempt"] == 2:
            retry = attempts[identity, 2]
            require(effect["committed_us"] < retry["committed_us"] <= job["settled_us"],
                    "retry lacks a committed second attempt after the first attempt's effect")
    return len(application_xids)


def verify_input(observed, profile, batches):
    checksums = {}
    for batch in batches:
        encoded = json.dumps(batch, indent=2, sort_keys=True, allow_nan=False) + "\n"
        checksums[batch["request_id"]] = digest(encoded.encode())
    require(observed == {"profile": profile, "batch_digests": checksums,
                         "input_sha256": digest("".join(checksums.values()).encode())},
            f"{profile}: archived input differs from the fixture")


def replay_arm(name, receipt, source, release, staged):
    files = read_archive(name)
    arm = json.loads(files["exit.json"])
    require(arm == receipt["status"] and arm["exit_code"] == 0 and arm["scratch_empty"] is True,
            f"{name}: exit status or cleanup differs")
    require(digest(files["SHA256SUMS"]) == receipt["raw_evidence_manifest_sha256"],
            f"{name}: raw evidence manifest differs from receipt")
    data = {Path(path).stem: json.loads(value) for path, value in files.items()
            if path.startswith("adapter/") and path.endswith(".json")}
    built = data["build"]
    require(built == staged["build"]
            and built["artifact_sha256"] == release["durable-jobs-apalis-bench"]
            and digest(files["adapter/build.json"]) == release["build.json"] == staged["build_manifest_sha256"]
            and digest(files["adapter/dependencies.txt"]) == release["dependencies.txt"] == built["cargo_tree_sha256"],
            f"{name}: archived build differs from the deployed release manifest")
    require(built["target"] == "x86_64" and "host: x86_64-unknown-linux-gnu" in built["rustc"]
            and "\nrelease: 1.99.0\n" in built["rustc"] and built["cargo"].startswith("cargo 1.99.0 "),
            f"{name}: unexpected Rust toolchain")
    runtime = data["runtime"]
    require(runtime["phase"] == "diagnostic" and runtime["pg_tls"] == "verify-full"
            and runtime["architecture"] == "x86_64" and runtime["platform"].startswith("Linux")
            and runtime["cpu_affinity"] == [0, 1], f"{name}: unexpected runtime")
    build_source = {f"tools/durable-jobs-bench/rust/{path}": expected
                    for path, expected in built["source_sha256"].items()}
    require(build_source | runtime["fixture_sha256"] == source, f"{name}: incomplete staged source identity")
    database_check(data["database"], 5)
    require(data["database"] == staged["database"]
            and b"starting PostgreSQL 17.11 on x86_64-pc-linux-gnu" in files["adapter/postgres.log"],
            f"{name}: observed PostgreSQL differs from the receipt")

    transactions = data["transactions"]["committed"]
    require(all(row["submitted_us"] <= row["committed_us"] <= row["returned_us"]
                for row in transactions), f"{name}: enqueue commit is outside its call")
    samples = data["samples"]["idle"] + data["samples"]["timed"]
    connections = [row for row in samples if "connections" in row]
    require(connections and all(row["all_tls"] and 0 < row["connections"] <= 5
                                and row["executing"] <= 1 and row["processes"] == 1
                                for row in connections), f"{name}: invalid connection or slot sample")
    raw = data["raw-writes"]
    pids = {row["worker_pid"] for row in raw["attempts"]}
    require(len(pids) == 1, f"{name}: expected one runner PID in attempt rows")
    data["application_xids"] = verify_rows(data["raw-jobs"], transactions, raw, pids)
    data["peak_connections"] = max(row["connections"] for row in connections)
    if "ledger_sha256" in receipt:
        require(digest(files["adapter/ledger.json"]) == receipt["ledger_sha256"],
                f"{name}: ledger digest differs from receipt")
    return data


def main():
    staged = json.loads((HERE.parent / "2026-11-23-native/apalis.json").read_bytes())
    source_bytes = (HERE / "source-v1.sha256").read_bytes()
    source = manifest(source_bytes)
    require(digest(source_bytes) == staged["source_allowlist_manifest_sha256"], "source receipt differs")
    historic_verifier = HERE / "staged-verify.py"
    for path, expected in source.items():
        actual = historic_verifier if path == "tools/durable-jobs-bench/verify.py" else ROOT / path
        require(digest(actual.read_bytes()) == expected, f"staged source differs: {path}")
    require(digest((HERE / "postgres-shared.sha256").read_bytes()) == staged["postgres_manifest_sha256"],
            "PostgreSQL install manifest differs from receipt")
    release = manifest((HERE / "release-v1.sha256").read_bytes())
    require(staged["artifact"] == {"artifact_sha256": release["durable-jobs-apalis-bench"],
                                   "version": adapter.VERSION}, "release artifact differs from receipt")

    functional = replay_arm("functional-v1", staged["functional"], source, release, staged)
    batches = adapter.smoke_batches()
    verify_input(functional["input"], "smoke", batches)
    transactions = functional["transactions"]
    require([tx["request_id"] for tx in transactions["committed"]] == ["success", "retry", "exhaust"]
            and transactions["rollback"]["request_id"] == "rollback"
            and transactions["rollback"]["enqueued_ids"].keys() == {"rollback"},
            "unexpected functional enqueue receipts")
    expected = {job["key"]: (digest(job["payload"].encode()), job["hold_ms"], job["fail_first"])
                for batch in batches if not batch["rollback"] for job in batch["jobs"]}
    require({job["job_key"]: (job["payload_sha256"], job["hold_ms"], job["fail_first"])
             for job in functional["raw-jobs"]} == expected, "functional jobs differ from fixture")
    adapter.check_smoke(functional["raw-jobs"], transactions["committed"], transactions["rollback"],
                        functional["raw-writes"])
    smoke = {"verified": True, "completed": 2, "failed": 1, "rolled_back": 1, "retried": 1}
    require(functional["smoke"] == smoke == {key: staged["functional"][key] for key in smoke},
            "functional smoke counts differ from receipt")
    require({key: len(rows) for key, rows in functional["raw-writes"].items()}
            == {"product": 3, "attempts": 4, "effects": 3}, "incorrect functional raw row counts")

    receipt = staged["trace_off"]
    sparse = replay_arm("sparse-trace-off-v1", receipt, source, release, staged)
    workload = adapter.SETTINGS["workloads"]["sparse"]
    verify_input(sparse["input"], "sparse", adapter.parity.inputs(workload, 1, adapter.SETTINGS["seed"]))
    ledger = sparse["ledger"]
    require(ledger["jobs"] == sparse["raw-jobs"] and ledger["transactions"] == sparse["transactions"]["committed"]
            and sparse["transactions"]["rollback"] is None and ledger["database"] == sparse["database"]
            and ledger["input_sha256"] == sparse["input"]["input_sha256"],
            "sparse ledger differs from raw evidence")
    historical = load_module("staged_verify", historic_verifier)
    historical.ROOT = ROOT
    require(historical.reconcile(ledger, "rust", "sparse", staged["artifact"]) == sparse["metrics"],
            "original sparse reconciliation differs from saved metrics")
    metrics = reconcile(ledger, "rust", "sparse", staged["artifact"])
    counts = {key: metrics[key] for key in ("completed", "failed", "ordered_independent_writes")}
    require(counts == {"completed": 24, "failed": 0, "ordered_independent_writes": 24}
            and all(job["native_state"] == "Done" for job in sparse["raw-jobs"])
            and {key: len(rows) for key, rows in sparse["raw-writes"].items()}
            == {"product": 24, "attempts": 24, "effects": 24}, "incorrect sparse outcome or raw row counts")
    require(sparse["application_xids"] == 48 and ledger["peak_connections"] == sparse["peak_connections"],
            "incorrect sparse XID count or connection peak")
    summary = dict(counts, application_write_xids=sparse["application_xids"],
                   enqueue_calls=len(ledger["transactions"]), peak_connections=sparse["peak_connections"],
                   runner_slots=ledger["slots"], input_sha256=ledger["input_sha256"])
    require(all(value == receipt[key] for key, value in summary.items()), "sparse evidence differs from receipt")
    print(json.dumps({"postgresql": sparse["database"]["version"], "functional": smoke, "sparse": summary},
                     indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
