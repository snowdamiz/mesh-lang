#!/usr/bin/env python3
"""Real PostgreSQL contention, fallback, mixed-version, and settlement gates."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack, closing
import json
from pathlib import Path
from threading import Barrier
from time import monotonic, sleep
import tempfile

from price import FIXTURE, WORKERS, Session, Wire, commands, module, setup

CLAIM = "SELECT * FROM mesh_jobs.claim_admitted_v3($1,$2,$3,$4,$5,$6)"
SETTLE = "SELECT mesh_jobs.settle_state($1,$2,$3,'{\"action\":\"complete\"}',false)"
COMPLETE = "SELECT * FROM mesh_jobs.complete_admitted_v3($1,$2,$3)"


def claim(db, ns, slots, cutoff=None):
    return db.call(CLAIM, (ns, "default", WORKERS, 60000, slots,
                          cutoff if cutoff is not None else 9223372036854775807))


def seed(db, ns, count, *, cap=4):
    db.call("INSERT INTO mesh_jobs.queues (namespace,name,concurrency) VALUES ($1,'default',$2)", (ns, cap))
    return db.call("""
        INSERT INTO mesh_jobs.jobs (namespace,worker,version,queue,args,metadata,tags,state,priority,
            max_attempts,timeout_ms,backoff_base_ms,backoff_max_ms,inserted_at_ms,scheduled_at_ms)
        SELECT $1,'parity',1,'default',jsonb_build_object('key',n),'{}','[]','available',
               9-n,3,60000,0,0,0,n
        FROM generate_series(1,$2::int) n RETURNING id::text""", (ns, count))


def args(rows):
    return [json.loads(row[3])["args"]["key"] for row in rows]


def pg_array(values):
    return "{" + ",".join(values) + "}"


def run(cluster, libpq, stock):
    wire = Wire(libpq)
    stock_claim, _ = commands(stock)
    with ExitStack() as stack:
        owner = stack.enter_context(closing(Session(wire, cluster.url("mesh", "parity_subject"))))
        legacy = stack.enter_context(closing(Session(wire, cluster.url("mesh", "admission_worker"))))
        modern = stack.enter_context(closing(Session(wire, cluster.url("mesh", "admission_worker"))))
        block = stack.enter_context(closing(Session(wire, cluster.url("mesh", "admission_worker"))))

        seed(owner, "mixed", 8)
        owner.call("UPDATE mesh_jobs.jobs SET worker='unregistered' WHERE namespace='mixed' AND priority=2")
        assert legacy.call("BEGIN ISOLATION LEVEL READ COMMITTED", tx=2) == "BEGIN"
        old = legacy.call(stock_claim, ("mixed", "default", WORKERS, 60000, 1, "", "false"), tx=2)
        assert args(old) == [8]
        # A second real backend queues behind the still-uncommitted v1 grant.
        start = Barrier(2)

        def waiting_grant():
            start.wait(timeout=2)
            return claim(modern, "mixed", 8)

        with ThreadPoolExecutor(max_workers=1) as executor:
            future = executor.submit(waiting_grant)
            start.wait(timeout=2)
            until = monotonic() + 0.08
            while True:
                waiting = cluster.sql("mesh", f"SELECT wait_event_type FROM pg_stat_activity WHERE pid={modern.pid}")
                if waiting == "Lock":
                    break
                assert monotonic() < until, waiting
                sleep(0.001)
            assert legacy.call("COMMIT") == "COMMIT"
            admitted = future.result(timeout=2)
        assert args(admitted) == [6, 5, 4]
        assert owner.call("SELECT count(*) FROM mesh_jobs.jobs WHERE namespace='mixed' AND state='executing'") == [("4",)]
        assert claim(modern, "mixed", 8) == []
        # Namespace pause is shared by new and old binaries.
        assert owner.call("SELECT mesh_jobs.set_namespace_paused('mixed',true)")[0][0]
        assert legacy.call(SETTLE, ("mixed", old[0][1], old[0][2])) == [("completed",)]
        assert claim(modern, "mixed", 8) == []
        assert owner.call("SELECT mesh_jobs.set_namespace_paused('mixed',false)")[0][0]
        refill = claim(modern, "mixed", 8)
        assert args(refill) == [3]

        # Race eight independently registered sessions for the same three
        # available slots. The first gets a committed grant and the remaining
        # requests see the capped count after owning the queue lock.
        seed(owner, "crowd", 8, cap=3)
        contenders = [
            stack.enter_context(closing(Session(wire, cluster.url("mesh", "admission_worker"))))
            for _ in range(8)
        ]
        start = Barrier(len(contenders))

        def contend(db):
            start.wait(timeout=3)
            return claim(db, "crowd", 2)

        with ThreadPoolExecutor(max_workers=len(contenders)) as executor:
            grants = list(executor.map(contend, contenders))
        assert sorted(sorted(args(rows)) for rows in grants if rows) == [[6], [7, 8]]
        assert owner.call("SELECT count(*) FROM mesh_jobs.jobs WHERE namespace='crowd' AND state='executing'") == [("3",)]

        # Hold the middle row, cancel a different row, and expire a fourth.
        # Available peers can commit and remain visible during the held lock.
        busy, cancelled, success, expired = admitted + refill
        block.call("BEGIN", tx=2)
        assert block.call("SELECT id FROM mesh_jobs.jobs WHERE namespace='mixed' AND id=$1 FOR NO KEY UPDATE",
                          (busy[1],), tx=2) == [(busy[1],)]
        owner.call("SELECT mesh_jobs.job_control('mixed',$1,'cancel',NULL,NULL)", (cancelled[1],))
        owner.call("UPDATE mesh_jobs.jobs SET lease_expires_at_ms=mesh_jobs.now_ms()-1 WHERE namespace='mixed' AND id=$1",
                   (expired[1],))
        cohort = [busy, cancelled, success, expired]
        result = modern.call(COMPLETE, ("mixed", pg_array([row[1] for row in cohort]),
                                        pg_array([row[2] for row in cohort])))
        assert result == [("3",)]
        assert owner.call("SELECT state FROM mesh_jobs.jobs WHERE namespace='mixed' AND id=$1",
                          (success[1],)) == [("completed",)]
        assert block.call("ROLLBACK") == "ROLLBACK"
        assert modern.call(SETTLE, ("mixed", busy[1], busy[2])) == [("completed",)]
        assert modern.call(SETTLE, ("mixed", cancelled[1], cancelled[2])) == [("cancelled",)]
        assert modern.call(SETTLE, ("mixed", expired[1], expired[2])) == [("",)]
        assert modern.call("SELECT mesh_jobs.recover('mixed',64,NULL)") == [("1",)]
        assert modern.call(SETTLE, ("mixed", expired[1], expired[2])) == [("",)]

        # Conservative size fallback precedes mutation. The released cursor
        # keeps the same ordered claim even with an incompressible value.
        seed(owner, "large", 4)
        owner.call("""UPDATE mesh_jobs.jobs SET args = args ||
            jsonb_build_object('blob',(SELECT string_agg(md5(g::text), '') FROM generate_series(1,4096) g))
            WHERE namespace='large' AND priority=5""")
        assert claim(modern, "large", 4)[0][0] == "fallback"
        assert owner.call("SELECT count(*) FROM mesh_jobs.jobs WHERE namespace='large' AND state='executing'") == [("0",)]
        legacy.call("BEGIN ISOLATION LEVEL READ COMMITTED", tx=2)
        direct = legacy.call(stock_claim, ("large", "default", WORKERS, 60000, 4, "", "false"), tx=2)
        legacy.call("COMMIT")
        assert args(direct) == [4, 3, 2, 1] and len(json.loads(direct[0][3])["args"]["blob"]) == 131072

        # The empty marker retains the exact earlier cutoff for the next-due
        # read, even if the eligible time has already crossed at receipt.
        seed(owner, "due", 2)
        assert claim(modern, "due", 4, cutoff=0) == [("empty", "", "", "", "0", "")]
        assert args(claim(modern, "due", 4, cutoff=1)) == [1]

        return dict(mixed_legacy_granted=1, mixed_v3_granted=len(admitted),
                    granted_keys=args(admitted), confirmed_refill_keys=args(refill),
                    independently_contending_runners=len(contenders), contention_cap=3,
                    paused_grants=0, cohort_visible_while_peer_locked=True,
                    cohort_cancelled_peer_used_scalar=True, expired_peer_recovered=True,
                    large_stock_fallback_bytes=131072, empty_cutoff_preserved=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("stock", "candidate", "libpq", "scratch", "output"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    args = parser.parse_args()
    h = module(FIXTURE)
    with tempfile.TemporaryDirectory(dir=args.scratch, prefix="edges-") as scratch:
        root = Path(scratch)
        cluster = h.Cluster(root, root, 96, expected_version="17.11")
        try:
            cluster.start()
            settings = setup(cluster, args.libpq, args.candidate, args.stock, ())
            evidence = run(cluster, args.libpq, args.stock)
        finally:
            cluster.stop()
    args.output.write_text(json.dumps(dict(settings=settings, **evidence), indent=2) + "\n")


if __name__ == "__main__":
    main()
