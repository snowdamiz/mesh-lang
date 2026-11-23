"""Reconcile PostgreSQL's committed product, attempt, effect, and settlement rows."""

import json


def epoch_us(value):
    return f"(extract(epoch FROM {value}) * 1000000)::bigint"


def commit_us(xid):
    return epoch_us(f"pg_xact_commit_timestamp({xid})")


def collect(cluster, engine, namespace, transactions, expected, relation, scope):
    sql_job = f"""
      SELECT j.id::text AS id, j.args->>'key' AS job_key, j.state, j.attempt,
             encode(sha256(convert_to(j.args->>'payload', 'UTF8')), 'hex') AS payload_sha256,
             (j.args->>'hold_ms')::integer AS hold_ms,
             (j.args->>'fail_first')::boolean AS fail_first,
             1 AS attempts, 1 AS effects, {epoch_us("a.entered_at")} AS entered_us,
             a.xmin::text AS attempt_xid, e.xmin::text AS effect_xid, j.xmin::text AS settled_xid,
             {commit_us("a.xmin")} AS attempt_committed_us,
             {commit_us("e.xmin")} AS effect_committed_us,
             {commit_us("j.xmin")} AS settled_us,
             {"j.attempted_at_ms * 1000" if engine == "mesh" else epoch_us("j.attempted_at")} AS claimed_us,
             a.worker_pid
      FROM {relation} j
      JOIN public.mesh_jobs_parity_attempts a ON a.job_key = j.args->>'key'
      JOIN public.mesh_jobs_parity_effects e
        ON (e.engine, e.namespace, e.job_key) = (a.engine, a.namespace, a.job_key)
      WHERE a.engine = '{engine}' AND a.namespace = '{namespace}' AND {scope}
    """
    jobs = json.loads(cluster.sql(engine, f"""
      SELECT coalesce(json_agg(w ORDER BY w.job_key), '[]') FROM ({sql_job}) w
    """))
    product = json.loads(cluster.sql(engine, f"""
      SELECT coalesce(json_agg(w), '[]') FROM (
        SELECT request_id, xmin::text AS xid, {commit_us("xmin")} AS committed_us
        FROM public.mesh_jobs_parity_product
        WHERE engine = '{engine}' AND namespace = '{namespace}'
      ) w
    """))

    # The base harness already checks payloads and counts *all* identities in
    # the database, including rows that wouldn't join a completed job.
    by_request = {item["request_id"]: item for item in product}
    if len(product) != len(by_request) or by_request.keys() != transactions.keys():
        raise ValueError("committed product transactions differ from enqueue receipts")
    accepted = {}
    receipts = []
    for request_id, receipt in transactions.items():
        committed = by_request[request_id]
        if int(receipt["xid"]) % (2**32) != int(committed["xid"]):
            raise ValueError(f"{request_id}: product transaction XID differs from receipt")
        if not receipt["submitted_us"] <= committed["committed_us"] <= receipt["acknowledged_us"]:
            raise ValueError(f"{request_id}: commit fell outside the producer call window")
        receipts.append(dict(receipt, **committed))
        for key, identity in receipt["enqueued_ids"].items():
            if key in accepted:
                raise ValueError(f"{key}: duplicate enqueue receipt")
            accepted[key] = (identity, committed["committed_us"], committed["xid"])

    if len(jobs) != len(expected) or {job["job_key"] for job in jobs} != expected.keys() or accepted.keys() != expected.keys():
        raise ValueError("ordered work identities differ from input")
    work_xids = set()
    for job in jobs:
        key = job["job_key"]
        identity, committed_us, enqueue_xid = accepted[key]
        xids = {job["attempt_xid"], job["effect_xid"], job["settled_xid"], enqueue_xid}
        if len(xids) != 4 or not all(xids) or job["id"] != identity or job["state"] != "completed":
            raise ValueError(f"{key}: mismatched identities or final state")
        entry_us, first_us, second_us, settled_us = (
            job[name] for name in ("entered_us", "attempt_committed_us", "effect_committed_us", "settled_us")
        )
        if not committed_us <= entry_us <= first_us < second_us <= settled_us:
            raise ValueError(f"{key}: enqueue, work, and settlement timestamps are out of order")
        if second_us - first_us < max(0, job["hold_ms"] - 1) * 1000:
            raise ValueError(f"{key}: committed hold was shorter than the frozen proof allows")
        work_xids.update((job["attempt_xid"], job["effect_xid"]))
    if len(work_xids) != 2 * len(jobs):
        raise ValueError("application records did not commit independently")
    return receipts, jobs
