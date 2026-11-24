"""Untimed local probe. Collect PostgreSQL and broker state if the shared reply watchdog fires."""
import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys

from proof import arm, backend, install, prove

DESCRIPTION = "shared producer/observer response"
OBSERVER_SQL = """
SELECT json_build_object(
  'committed_jobs', (SELECT count(*) FROM mesh_jobs.jobs),
  'producer', (SELECT json_build_object(
    'state', state, 'wait_event_type', wait_event_type, 'wait_event', wait_event,
    'query_class', CASE
       WHEN query LIKE '%enqueue_batch_v1(%' THEN 'bulk enqueue'
       WHEN query LIKE '%mesh_jobs.enqueue(%' THEN 'scalar enqueue'
       WHEN query LIKE '%parity_product%' THEN 'producer marker'
       WHEN query LIKE 'COMMIT%' THEN 'commit'
       ELSE 'other' END,
    'transaction_open', xact_start IS NOT NULL)
   FROM pg_stat_activity WHERE usename = 'parity_producer' AND datname = 'mesh')
)"""


def read_state(cluster):
    args, env = cluster.psql("mesh")
    data = subprocess.check_output([*args, "-c", OBSERVER_SQL], env=env, text=True, timeout=5)
    return json.loads(data)


@contextmanager
def capture(out, *, block_marker):
    wait, launch = arm.h.wait_for, subprocess.Popen
    child = None
    cluster = None
    fence = None

    class Cluster(backend.Cluster):
        def start(self):
            nonlocal cluster
            super().start()
            cluster = self

    def launched(*args, **kwargs):
        nonlocal child, fence
        broker = args and isinstance(args[0], list) and args[0][-1] == "session"
        if broker and block_marker:
            command, env = cluster.psql("mesh")
            fence = launch(command, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                           stderr=subprocess.PIPE, text=True)
            fence.stdin.write("BEGIN;\nLOCK TABLE public.mesh_jobs_parity_product IN ACCESS EXCLUSIVE MODE;\n"
                              "\\echo lock-acquired\n")
            fence.stdin.flush()
            assert fence.stdout.readline().strip() == "lock-acquired"
        process = launch(*args, **kwargs)
        if broker:
            child = process
        return process

    def watched(probe, deadline, description, *args):
        try:
            return wait(probe, deadline, description, *args)
        except arm.h.FixtureError:
            if description == DESCRIPTION and child is not None and cluster is not None:
                replies = (out / "broker-output.log").read_text().splitlines()
                witness = dict(process_alive=child.poll() is None, complete_replies=len(replies),
                               setup_ready=replies[:1] == ["SESSION_READY"], database=read_state(cluster),
                               receipt_written=(out / "enqueue.log").exists())
                if witness["process_alive"] and platform.system() == "Darwin":
                    subprocess.run(["sample", str(child.pid), "1", "3", "-file", str(out / "broker-sample.txt")],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
                    sample_file = out / "broker-sample.txt"
                    if sample_file.is_file():
                        sample = sample_file.read_text()
                        witness["stack_symbols"] = [expr for expr in
                            ("mesh_jobs_enqueue_in", "mesh_json_parse", "serde_json", "mesh_io_read_line",
                             "mesh_pg", "poll") if re.search(re.escape(expr), sample)]
                witness["controlled_marker_lock"] = block_marker
                arm.h.write_json(out / "transport-failure.json", witness)
                if block_marker:
                    # Prove the blocked producer transaction aborts before freeing the held lock.
                    child.kill()
                    child.wait(timeout=5)
                    assert read_state(cluster)["committed_jobs"] == 0
                    fence.stdin.write("ROLLBACK;\n\\q\n")
                    fence.stdin.flush()
                    fence.wait(timeout=5)
            raise

    prior = backend.Cluster
    backend.Cluster, arm.h.wait_for, subprocess.Popen = Cluster, watched, launched
    try:
        yield
    finally:
        backend.Cluster, arm.h.wait_for, subprocess.Popen = prior, wait, launch
        if fence is not None and fence.poll() is None:
            fence.kill()
            fence.wait(timeout=5)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--block-marker", action="store_true")
    parser.add_argument("destination", type=Path)
    parser.add_argument("runner", type=Path)
    parser.add_argument("producer", type=Path)
    args = parser.parse_args()
    destination, runner, producer = (p.resolve() for p in (args.destination, args.runner, args.producer))
    os.environ.update(PRODUCT_COMPOSITION="product-group", PRODUCT_STORAGE="default",
                      PRODUCT_TOPOLOGY="slow-equal-cap71")
    install()
    with capture(destination, block_marker=args.block_marker):
        prove(destination, runner, producer)
