"""Pair the pinned V1 consumers with either the stock or true-bulk broker."""

import argparse
from contextlib import ExitStack, contextmanager
import hashlib
import json
import os
from pathlib import Path

import arm
import identity

STATEMENTS = """
SELECT coalesce(json_object_agg(query, calls), '{}') FROM pg_stat_statements
WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database())
  AND userid = 'parity_producer'::regrole
  AND (query LIKE 'SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(%'
    OR query LIKE 'SELECT mesh_jobs.enqueue(%')
"""


def run(source, out, producer, mode, local):
    load_stock = arm.load_arm
    runner = Path(os.environ["NATIVE_JOBS_ARTIFACTS"]) / "mesh"
    runner_sha, producer_sha = arm.sha(runner), arm.sha(producer)

    def load(source):
        stock = load_stock(source)
        base_subject, base_cluster = stock.Subject, stock.Cluster

        class Subject(base_subject):
            def call(self, action, env, log):
                # Both arms install exactly the same additive bulk-capable schema.
                original = self.command
                if action == "migrate":
                    self.command = [str(producer)]
                try:
                    return super().call(action, env, log)
                finally:
                    self.command = original

            @contextmanager
            def warm_producer(self, env, evidence, deadline):
                # The stock harness starts the broker in __enter__. Restore the
                # runner executable before it launches either worker process.
                with ExitStack() as broker:
                    original = self.command
                    if mode == "bulk":
                        self.command = [str(producer)]
                        env = dict(env, MESH_JOBS_SET_BASED_ENQUEUE="1")
                    try:
                        enqueue = broker.enter_context(super().warm_producer(env, evidence, deadline))
                    finally:
                        self.command = original
                    yield enqueue

            def run(self, env, index, log):
                stamp = identity.pinned_file(runner, runner_sha)
                process = super().run(env, index, log)
                try:
                    if index == 0:
                        self.starts = {}
                    self.starts[process] = (stamp, identity.process_stamp(process))
                except BaseException:
                    stock.h.stop_runners([process], require_clean=False)
                    raise
                return process

            def capture_runners(self, processes, logs):
                if len(processes) != 2 or set(processes) != self.starts.keys():
                    raise ValueError("expected the two directly launched runners")
                identities = [identity.observe(process, runner, "run", runner_sha, *self.starts[process], [0, 1])
                              for process in processes]
                arm.write(out / "runner-identities.json", dict(adapter_pid=os.getpid(), runners=identities))
                super().capture_runners(processes, logs)

        class Cluster(base_cluster):
            def reconcile_trial(self, engine, namespace, enqueued_ids, handler_attempts, commits):
                result = super().reconcile_trial(engine, namespace, enqueued_ids, handler_attempts, commits)
                statements = json.loads(self.sql(engine, STATEMENTS))
                bulk = sum(calls for sql, calls in statements.items() if sql.startswith("SELECT ordinal, receipt"))
                scalar = sum(calls for sql, calls in statements.items() if sql.startswith("SELECT mesh_jobs.enqueue("))
                arm.write(out / "producer-writer.json", dict(mode=mode,
                    runner_sha256=runner_sha, producer_sha256=producer_sha if mode == "bulk" else runner_sha,
                    migration_sha256=producer_sha, bulk_calls=bulk, scalar_calls=scalar, statements=statements))
                if (bulk, scalar) != ((8, 0) if mode == "bulk" else (0, 1024)):
                    raise ValueError(f"unexpected producer SQL mix: {bulk} bulk and {scalar} scalar calls")
                return result

        stock.Subject, stock.Cluster = Subject, Cluster
        return stock

    arm.load_arm = load
    arm.run(source, "tuned-2x32", out, local, False)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("out", type=Path)
    parser.add_argument("producer", type=Path)
    parser.add_argument("mode", choices=("scalar", "bulk"))
    parser.add_argument("--local", action="store_true")
    args = parser.parse_args()
    run(args.source, args.out, args.producer, args.mode, args.local)
