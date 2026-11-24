"""Check split identities, independent writes, checkout probes and short clocks."""
import hashlib
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[4]
RESULTS = HERE.parents[1]
REPAIR = RESULTS / "2026-11-23-ordinary-completion-writer-native-repair"
sys.path.insert(0, str(REPAIR / "replay"))
try:
    from archived import fault_checks as check_earlier_faults
    from proof import check as check_r2
finally:
    sys.path.pop(0)

INPUT = "ac0bcbf2465ea6d6807e8bfcdcfc3b1dacfa18b775e3f4c074257d66bfbba9d1"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def manifest(directory):
    path = directory / "mesh.json"
    build = json.loads(path.read_text())
    assert digest(directory / "mesh") == build["files"]["mesh"]
    return dict(elf=build["files"]["mesh"], manifest=digest(path), version=build["version"])


def check_arm(read, checkout, runner, *, native, historical=False):
    evidence = check_r2(read, "off", runner, native=native)
    split = read("split-executables.json")
    assert historical or split["checkout_mode"] == checkout
    row, ledger, db, queue, statements = map(read, (
        "row.json", "ledger.json", "postgres-settings.json", "queue-policy.json",
        "trusted-application-statements.json"))
    assert row["profile"] == "short" and row["runners"] == 1 and row["input_sha256"] == INPUT
    assert row["completed"] == row["reconciled_jobs"] == 128
    assert all(job["state"] == "completed" for job in ledger["jobs"])
    assert row["queue_policy"] == ledger["queue_policy"] == queue["queue_policy"] == "success_cohort"
    assert queue["admitted_queries"] > 0
    assert db["version"] == "17.11" and db["database_connection_limit"] == 20
    assert all(db[k] for k in ("fsync", "full_page_writes", "synchronous_commit", "tls", "track_commit_timestamp"))
    for suffix in ("attempts", "effects"):
        (stmt,) = [item for item in statements
                   if item["query"].startswith("INSERT INTO public.mesh_jobs_parity_" + suffix)]
        assert stmt["calls"] == stmt["rows"] == 128
    probes = sum(item["calls"] for item in statements if item["query"] == "SELECT $1")
    if checkout == "d2":
        assert 0 <= probes < 256, f"D2 did not reduce application checkout probes: {probes}"
    else:
        assert probes == 256, f"expected 256 application checkout probes, got {probes}"
    (batch,) = ledger["transactions"]
    enqueue_xid = str(int(batch["xid"]) % (2**32))
    assert all(enqueue_xid not in {job[k] for k in ("attempt_xid", "effect_xid", "settled_xid")}
               for job in ledger["jobs"])
    settled = max(job["settled_us"] for job in ledger["jobs"])
    assert batch["submitted_us"] <= batch["committed_us"] < settled
    return dict(evidence, checkout_probes=probes, complete=128, errors=0,
                submit_jobs_per_s=128_000_000 / (settled - batch["submitted_us"]),
                commit_jobs_per_s=128_000_000 / (settled - batch["committed_us"]))


def check_runner_log(text):
    status = json.loads(text.strip().splitlines()[-1])
    assert status["state"] == "stopped" and status["drained"] and status["last_error"] is None
    assert status["claimed"] == status["finished"] == status["outcomes"]["completed"] == 128
    assert all(total == 0 for outcome, total in status["outcomes"].items() if outcome != "completed")
    assert status["recovered"] == status["stale_acks"] == status["undrained"] == 0


def check_teardown(receipt, mode):
    assert receipt["mode"] == mode
    held = receipt["held_callback_state"]
    assert held["state"] == "executing" and held["attempt"] == 1
    assert len({held[k] for k in ("job_xid", "attempt_xid", "effect_xid")}) == 3
    if mode == "d2-forced":
        assert receipt["process_exit"] == -9 and receipt["after_forced_exit_state"] == held
    else:
        done = receipt["after_callback_state"]
        assert done["state"] == "completed" and done["attempt"] == 1 and receipt["normal_exit"]
        assert all(held[k] == done[k] for k in ("attempt_xid", "effect_xid"))
        assert held["job_xid"] != done["job_xid"]


def check_faults(read, runner_sha, broker_sha, checkout="d2"):
    cases = ("lost-application-reply", "lost-claim-reply", "lost-terminal-reply")
    if checkout == "d2":
        cases = ("stale-session", *cases)
    for case in cases:
        recorded = read(f"{case}/result.json")
        assert (recorded["case"], recorded["checkout"], recorded["runner_sha256"], recorded["broker_sha256"]) == (
            case, checkout, runner_sha, broker_sha)
        fault, requests = recorded["result"], recorded["requests"]
        if case == "stale-session":
            retired, replacement = fault["retired_port"], fault["replacement_port"]
            assert retired != replacement
            assert [sql for port, sql in requests if port == retired] == ["SELECT 1"]
            assert sum(port == replacement and sql.startswith("INSERT") for port, sql in requests) == 2
            assert fault["final"]["state"] == "completed"
        elif case == "lost-application-reply":
            original, replacement = fault["original_port"], fault["replacement_port"]
            assert original != replacement and fault["retransmitted_application_writes"] == 0
            writes = [(port, sql) for port, sql in requests if sql.startswith("INSERT")]
            assert [port for port, _ in writes] == [original, replacement, replacement]
            assert "parity_attempts" in writes[0][1] and "parity_effects" in writes[-1][1]
            assert fault["before"]["attempts"] == fault["final"]["attempts"]
            assert fault["before"]["state"] == "executing" and fault["final"]["state"] == "discarded"
            assert fault["final"]["effect_xid"] is None and fault["next_job"]["state"] == "completed"
            assert fault["wire"] == dict(command="INSERT 0 1", rows=0, idle=True, receipt_dropped=True)
        else:
            before, after = fault["before"], fault["final"]
            assert fault["wire"]["receipt_dropped"] and fault["wire"]["idle"]
            assert after["state"] == fault["next_job"]["state"] == "completed"
            if case == "lost-claim-reply":
                assert (before["state"], before["attempt"], before["attempts"], after["attempt"]) == (
                    "executing", 1, [], 2)
                assert fault["wire"]["command"] == "COMMIT"
                assert fault["recovered_attempt_commit_ms"] >= before["lease_expires_at_ms"]
            else:
                assert before == after and fault["wire"]["rows"] == 1
    return list(cases)
