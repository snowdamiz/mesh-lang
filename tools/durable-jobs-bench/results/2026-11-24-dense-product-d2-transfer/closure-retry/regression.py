"""Exercise the actual old and installed adapter logic on native short receipts."""
import importlib.util
import json
import os
from pathlib import Path
import sys
import tarfile
from tempfile import TemporaryDirectory
from unittest.mock import patch

import amendment as a

d = a.dense
FROZEN = d.RESULTS / "2026-11-23-set-based-enqueue/native/bulk-enqueue-ce5e551d079e.tar.gz"
FP = d.FROZEN
FILES = ("tools/durable-jobs-bench/native/arm.py", "tools/durable-jobs-bench/native/ledger.py",
         "proof/jobs-parity/linux-regression/trial.py", "proof/jobs-parity/linux-regression/oban_runtime.py",
         "proof/jobs-parity/linux-regression/proof_transport.py", "scripts/jobs-parity.py",
         "scripts/jobs-parity-profiles.json", "scripts/lib/jobs_parity_fault.py")


def extract_frozen(path):
    stock = a.run.read_json(d.RESULTS / "2026-11-24-bulk-d2-short-peers/short.json")
    pins = stock["lanes"]["bulk"]["inventory"]["engines"]["mesh"]["verified_files"]
    with tarfile.open(FROZEN, "r:gz") as raw:
        read = d.archive_files(raw, "ordinary-v2/")
        for name in FILES:
            contents = read(name)
            expected = pins.get(str(FP / name))
            if expected is not None:
                assert d.digest(contents) == expected, name
            file = path / name
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_bytes(contents)


def load(path):
    spec = importlib.util.spec_from_file_location(path.stem, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def fixture():
    first = min((a.SHORT / "blocks").iterdir())
    raw = first / "measured-product-group/measured-mesh/raw.tar.gz"
    with tarfile.open(raw, "r:gz") as archive:
        read = d.archive_files(archive, "adapter/")
        return {name: json.loads(read(f"{name}.json")) for name in
                ("application-records", "ledger", "queue-policy", "trusted-application-statements")}


def exercise(frozen, scratch):
    original = a.frozen_adapter()
    for name in ("identity.py", "backend.py"):
        (scratch / name).write_bytes(original[name])
    sys.path.insert(0, str(scratch))
    os.environ["PRODUCT_FROZEN_HARNESS"] = str(frozen)
    old = load(scratch / "backend.py")
    new = load(a.PUBLIC / "closure-retry/backend.py")
    assert Path(old.arm.__file__) == frozen / FILES[0] and old.arm is new.arm
    arm = old.arm
    ledger = fixture()
    jobs, transactions = [ledger["ledger"][name] for name in ("jobs", "transactions")]
    queue = ledger["queue-policy"]
    sql = {
        "mesh_jobs_parity_attempts": ledger["application-records"],
        "parity_subject": queue["queue_statements"],
        "compiled_caller": ledger["trusted-application-statements"],
    }
    namespace = ledger["application-records"]["attempts"][0]["namespace"]
    inherited = arm.Cluster.reconcile_trial
    receipts = []

    def run(backend, out):
        arm.Cluster.reconcile_trial = inherited
        backend.install()
        cluster = backend.Cluster.__new__(backend.Cluster)
        cluster.evidence = out
        cluster.evidence.mkdir()
        cluster.trusted_application = True
        cluster.runner_pids = {job["worker_pid"] for job in jobs}
        cluster.transactions = {}
        cluster.queue_policy = "success_cohort"
        cluster.sql = lambda _engine, query: json.dumps(sql[next(key for key in sql if key in query)])
        return cluster.reconcile_trial("mesh", namespace, {}, {}, {})

    def base(cluster, engine, namespace, *args):
        receipts.append((engine, namespace))
        return 128

    with patch.object(arm.regression.Cluster, "reconcile_trial", base), \
         patch.object(arm.ledger, "collect", lambda *_: (transactions, jobs)):
        try:
            run(old, scratch / "failed")
        except RuntimeError as error:
            assert str(error) == "super(): __class__ cell not found", str(error)
        else:
            raise AssertionError("original adapter did not reproduce the class-cell failure")
        assert not receipts
        assert run(new, scratch / "recovered") == 128
        assert receipts == [("mesh", namespace)]
        assert a.run.read_json(scratch / "recovered/queue-policy.json") == queue
        assert a.run.read_json(scratch / "recovered/trusted-application-statements.json") == sql["compiled_caller"]
        sql["parity_subject"] = [
            row for row in sql["parity_subject"] if "FROM mesh_jobs.claim_batch_set_v1(" not in row["query"]
        ]
        try:
            run(new, scratch / "bad-claim")
        except RuntimeError as error:
            assert "database counters disagree" in str(error)
        else:
            raise AssertionError("repaired adapter accepted missing checked-admission evidence")


def replay_recovery(scratch):
    groups = a.short_replay()
    old = a.PUBLIC / "evidence/native/timing/short-smaller/blocks"
    failed = min(old.glob("01-short.*"))
    repaired = a.SHORT / "blocks" / groups["1"]["attempt"]
    with tarfile.open(failed / "measured-product-bulk/measured-mesh/raw.tar.gz", "r:gz") as raw:
        failure = a.run.file_json(raw, "adapter/failure.json")
        assert failure["type"] == "RuntimeError"
        assert failure["message"] == "super(): __class__ cell not found"
        assert a.run.file_json(raw, "status.json")["exit_code"] != 0

    (scratch / "blocks").mkdir()
    for folder in (failed, repaired):
        (scratch / "blocks" / folder.name).symlink_to(folder, target_is_directory=True)
    block = a.run.read_json(repaired / "order.json")
    source = a.PUBLIC / "replay/intake.py"
    scorer = load(source)
    bound = a.run.read_json(a.SHORT_ROOT / "short-smaller.inventory.json")

    def compare(path, block, *_):
        return scorer.compare(bound, path, block["group"])

    failures = {}
    with patch.object(a.matrix, "replay_block", compare):
        chosen = a.matrix.pick(scratch, block, {}, {}, failures)
    assert failed.name in failures and chosen[0] == repaired.name
    assert {lane: row["raw_sha256"] for lane, row in chosen[1].items()} == groups["1"]["raw"]


def main():
    with TemporaryDirectory(prefix="closure-regression-", dir=Path(sys.argv[1])) as scratch:
        scratch = Path(scratch)
        (scratch / "drivers").mkdir()
        extract_frozen(scratch / "frozen")
        exercise(scratch / "frozen", scratch / "drivers")
        replay_recovery(scratch)
    print("Archived original closure failure reproduced. Installed repair reconciled native short SQL, rejected missing admission, and recovered the failed pilot from the selected real raw attempt.")


if __name__ == "__main__":
    main()
