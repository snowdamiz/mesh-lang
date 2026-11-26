"""Verify matching independent write, terminal, checkout, and executable evidence."""
from collections import Counter

BULK_MANIFEST = "7f228c192630bb6d95cbf9e06c5e0132931f429c1cfa07391bb28cb6fdec8911"
BULK_ELF = "55d7000d562a6ad395f5e64e33e0e973fac0084ed05ad6c30a33db2a3fc6303a"


def check(read, mode, runner, *, native):
    split, row, ledger, writer, policy, tls = map(read, (
        "split-executables.json", "row.json", "ledger.json", "producer-writer.json",
        "queue-policy.json", "postgres-tls-sessions.json"))
    assert split["producer"] == split["migrator"] != split["runner"]
    assert split["writer_mode"] == mode and split["runner_sha256"] == runner["elf"]
    assert split["runner_manifest_sha256"] == runner["manifest"] and split["runner_version"] == runner["version"]
    if native:
        assert split["broker_manifest_sha256"] == BULK_MANIFEST and split["broker_sha256"] == BULK_ELF
        assert not row["local_smoke"]
    assert row["artifact_sha256"] == split["broker_manifest_sha256"]
    assert writer["enabled"] and writer["bulk"] == 1 and writer["scalar"] == 0
    assert row["completed"] == len(ledger["jobs"]) == 128
    assert row["committed_work_records"] == 256 and row["peak_connections"] == 13
    assert row["slots"] == 16 and row["connection_cap"] == 20 and 0 <= row["peak_executing"] <= 16
    assert split["warmed_application_sessions"] == 8
    assert all(session["tls"] for session in tls)
    roles = Counter(item["role"] for item in tls)
    assert roles["compiled_caller"] == 8 and roles["parity_producer"] == 1
    assert 2 <= roles["parity_subject"] <= 4
    assert len(ledger["transactions"]) == 1
    app = {job[k] for job in ledger["jobs"] for k in ("attempt_xid", "effect_xid")}
    terminal = Counter(job["settled_xid"] for job in ledger["jobs"])
    assert len(app) == 256 and not (terminal.keys() & app)
    assert all(job["attempt"] == job["effects"] == job["attempts"] == 1 for job in ledger["jobs"])
    assert all(job["attempt_committed_us"] < job["effect_committed_us"] <= job["settled_us"] for job in ledger["jobs"])
    assert len(terminal) == policy["terminal_xids"] and max(terminal.values()) <= 8
    statements = [line for line in policy["queue_statements"] if
                  "mesh_jobs.settle_state(" in line["query"] or "mesh_jobs.complete_admitted_v3(" in line["query"]]
    assert sum(line["rows"] for line in statements) == 128
    assert sum(line["calls"] for line in statements) == len(terminal)
    return dict(terminal_groups_by_size=Counter(terminal.values()), terminal_xids=len(terminal),
                warmed_application_sessions=split["warmed_application_sessions"])
