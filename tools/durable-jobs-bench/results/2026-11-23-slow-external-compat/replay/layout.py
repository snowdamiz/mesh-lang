"""Install and prove the isolated one-function compatibility overlay."""
from collections import Counter
import hashlib
import json
from pathlib import Path

import storage

HERE = Path(__file__).resolve().parent
INSTALL = HERE / "install-layout.sql"
if not INSTALL.is_file():
    INSTALL = HERE.parent / "frozen/install-layout.sql"

FUNCTIONS = """
    SELECT jsonb_object_agg(p.proname, jsonb_build_object(
      'oid', p.oid, 'owner', pg_get_userbyid(p.proowner), 'acl', p.proacl,
      'definer', p.prosecdef, 'config', p.proconfig, 'volatility', p.provolatile,
      'body_md5', md5(p.prosrc)))
    FROM pg_proc p
    WHERE p.oid IN ('mesh_jobs.enqueue_batch_layout_v1()'::regprocedure,
                    'mesh_jobs.enqueue_batch_v1(text,jsonb)'::regprocedure)
"""
CAPABILITY = "SELECT mesh_jobs.enqueue_batch_layout_v1()"
RECEIPTS = """
    SELECT coalesce(jsonb_agg(to_jsonb(w) ORDER BY w.request_key), '[]')
    FROM (
      SELECT j.position, j.id::text, r.request_key, r.fingerprint, r.receipt,
             floor(extract(epoch FROM pg_xact_commit_timestamp(r.xmin))*1000000)::bigint AS committed_us,
             r.xmin::text AS xid
      FROM mesh_jobs.receipts r
      JOIN mesh_jobs.jobs j ON j.namespace = r.namespace AND j.id::text = r.receipt->>'id'
    ) w
"""


def install(cluster, database, external, compatible):
    before = json.loads(cluster.sql(database, FUNCTIONS))
    if cluster.sql(database, CAPABILITY) != "t":
        raise ValueError("expected the original migrated EXTENDED layout")
    if {function["owner"] for function in before.values()} != {"parity_subject"}:
        raise ValueError("fixture queue owner changed")
    commands = ["BEGIN", "SET LOCAL ROLE parity_subject"]
    if compatible:
        commands.append(INSTALL.read_text())
    if external:
        commands.append("ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTERNAL")
    cluster.sql(database, ";\n".join([*commands, "COMMIT"]))
    after = json.loads(cluster.sql(database, FUNCTIONS))
    expected = "t" if compatible or not external else "f"
    if cluster.sql(database, CAPABILITY) != expected:
        raise ValueError("installed guard classified the physical layout incorrectly")
    return dict(before=before, after=after, guard_pass=expected == "t",
                installed_sha256=storage.sha(INSTALL.read_bytes()) if compatible else None)


def check_install(witness, external, compatible):
    before, after = (witness[key] for key in ("before", "after"))
    if before.keys() != {"enqueue_batch_layout_v1", "enqueue_batch_v1"} or after.keys() != before.keys():
        raise ValueError("installer modified the function inventory")
    if before["enqueue_batch_v1"] != after["enqueue_batch_v1"]:
        raise ValueError("installer modified the batch writer")
    body_before = before["enqueue_batch_layout_v1"]
    expected_body = body_before["body_md5"]
    if compatible:
        expected_body = hashlib.md5(INSTALL.read_text().split("$$", 2)[1].encode()).hexdigest()
    if after["enqueue_batch_layout_v1"] != dict(body_before, body_md5=expected_body):
        raise ValueError("layout helper differs from the sealed body or changed ownership or ACLs")
    expected_hash = storage.sha(INSTALL.read_bytes()) if compatible else None
    if (witness["installed_sha256"], witness["guard_pass"]) != (expected_hash, compatible or not external):
        raise ValueError("unexpected installer or admission")


def verify_receipts_and_slots(saved, transaction, batches, jobs, require_peak=False):
    entries = {row["request_key"]: row for row in saved}
    frozen = [job for batch in batches for job in batch["jobs"]]
    if len(saved) != len(entries) or entries.keys() != transaction["enqueued_ids"].keys():
        raise ValueError("durable request keys differ from the acknowledged enqueue IDs")
    for position, job in enumerate(frozen, 1):
        row = entries[job["key"]]
        identity = transaction["enqueued_ids"][job["key"]]
        want = dict(id=identity, effect_key=identity + "/1", inserted=True,
                    conflict=False, state="available")
        spec = dict(worker="parity", version=1, args=job, queue="default", metadata={},
                    tags=[], priority=0, scheduled_at_ms=None, delay_ms=None, max_attempts=3,
                    timeout_ms=60000, backoff_base_ms=1000, backoff_max_ms=1000,
                    suspended=False, unique=None, replace={})
        fingerprint = storage.sha(json.dumps(spec, sort_keys=True, separators=(",", ":"),
                                             ensure_ascii=False).encode())
        if (row["id"], row["position"], row["receipt"], row["fingerprint"],
            row["committed_us"]) != (identity, position, want, fingerprint, transaction["committed_us"]):
            raise ValueError(f"{job['key']}: persisted enqueue receipt differed from the input and commit")
    # Each enqueue_in group owns a savepoint. Its receipt rows carry that
    # subtransaction XID and the same durable commit timestamp as the parent.
    subtransactions = Counter(row["xid"] for row in saved)
    if transaction["xid"] in subtransactions or sorted(subtransactions.values()) != [8] + [63] * 8:
        raise ValueError("unexpected receipt savepoint grouping")
    events = sorted((row[clock], delta) for row in jobs
                    for clock, delta in (("attempt_committed_us", 1), ("effect_committed_us", -1)))
    occupied = peak = 0
    for _, delta in events:
        occupied += delta
        peak = max(occupied, peak)
    if occupied != 0 or peak > 64 or (require_peak and peak != 64):
        raise ValueError(f"slow lane must witness all 64 simultaneous committed holds, saw {peak}")
    return peak

