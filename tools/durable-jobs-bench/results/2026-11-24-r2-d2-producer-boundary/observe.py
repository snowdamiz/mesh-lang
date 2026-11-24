"""Scope additional clocks and PostgreSQL statistics to the real producer."""
import ast
from contextlib import contextmanager
from copy import copy
from hashlib import sha256
import json
import os
from pathlib import Path
from textwrap import dedent
import time

from adapter import trial_source


class Clock:
    @staticmethod
    def clock_pair():
        before = time.monotonic_ns()
        wall = time.time_ns()
        return [before, wall, time.monotonic_ns()]

    def __init__(self):
        self.before = self.clock_pair()
        self.events = {}

    def mark(self, name):
        assert name not in self.events
        self.events[name] = time.monotonic_ns()

    def finish(self):
        return dict(before=self.before, after=self.clock_pair(), events=self.events)


STATEMENTS = """
SELECT coalesce(json_agg(row_to_json(p) ORDER BY p.toplevel DESC, p.queryid), '[]')
FROM (
    SELECT queryid, query, calls, rows, toplevel, total_plan_time, total_exec_time,
           shared_blks_hit, shared_blks_read, temp_blks_written, wal_bytes
    FROM pg_stat_statements
    WHERE userid = 'parity_producer'::regrole AND dbid = (SELECT oid FROM pg_database WHERE datname = 'mesh')
) p
"""

FUNCTIONS = """
SELECT coalesce(json_agg(row_to_json(f) ORDER BY f.funcid), '[]')
FROM (
    SELECT funcid, schemaname, funcname, calls, total_time, self_time
    FROM pg_stat_user_functions WHERE schemaname = 'mesh_jobs'
) f
"""


def install(arm, subject):
    mode = os.environ["R2_OBSERVATION"]
    if mode not in {"client", "nested"}:
        raise ValueError("invalid observation mode")

    # Keep the original context-manager protocol and the identity hook
    # installed by the split adapter, changing only this direct producer spawn.
    original = arm.regression.Subject.warm_producer.__wrapped__
    # The identity hook compiles a dynamic copy. Read the pinned original on disk
    # and reproduce the same Popen hook before patching the caller boundaries.
    path = Path(original.__code__.co_filename)
    assert sha256(path.read_bytes()).hexdigest() == "db8e121ab9b1f6aed2293b26c79a4fbfa8ac8aba6ff228be54b360bd32048a8e"
    source = path.read_text()
    syntax = ast.parse(source)
    cls, = [node for node in syntax.body if getattr(node, "name", "") == "Subject"]
    fn, = [node for node in cls.body if getattr(node, "name", "") == "warm_producer"]
    start, = fn.decorator_list
    source = dedent("".join(source.splitlines(keepends=True)[start.lineno - 1:fn.end_lineno]))
    anchor = '    ) as broker, output.open("rb") as reader:\n'
    assert source.count(anchor) == 1
    source = trial_source(source.replace(anchor, anchor + "        self.observe_broker_launch(broker)\n"))
    scope = dict(original.__globals__, Clock=Clock)
    exec(compile(source, path, "exec"), scope)
    arm.regression.Subject.warm_producer = scope["warm_producer"]

    stock_settings = arm.database_settings

    def settings(cluster, engine):
        settings = stock_settings(cluster, engine)
        arm.h.write_json(cluster.evidence / "caller-stage.json", cluster.caller_stage)
        config = json.loads(cluster.sql(engine, """
            SELECT coalesce(json_agg(row_to_json(s)), '[]') FROM (
                SELECT setconfig FROM pg_db_role_setting
                WHERE setrole = 'parity_producer'::regrole
                  AND setdatabase = (SELECT oid FROM pg_database WHERE datname = 'mesh')
            ) s
        """))
        arm.h.write_json(cluster.evidence / "producer-sql.json", dict(
            mode=mode, role_settings=config, statements=json.loads(cluster.sql(engine, STATEMENTS)),
            functions=json.loads(cluster.sql(engine, FUNCTIONS))))
        return settings

    class ObservedCluster(arm.Cluster):
        def prepare_database(self, engine):
            super().prepare_database(engine)
            if mode == "nested":
                self.sql("postgres", """
                    ALTER ROLE parity_producer IN DATABASE mesh SET pg_stat_statements.track = 'all';
                    ALTER ROLE parity_producer IN DATABASE mesh SET track_functions = 'pl';
                """)

    base_subject = subject.__bases__[0]

    class ObservedSubject(subject):
        def __init__(self, *args, **kwargs):
            super().__init__(*args, **kwargs)
            self.probe_broker = Path(os.environ["R2_OBSERVED_BROKER"]).resolve(strict=True)
            self.probe_sha = arm.h.digest(self.probe_broker)
            self.probe_manifest_sha = arm.h.digest(self.probe_broker.with_name("mesh.json"))
            assert self.probe_sha == os.environ["R2_OBSERVED_BROKER_SHA"]
            assert self.probe_manifest_sha == os.environ["R2_OBSERVED_MANIFEST_SHA"]

        @contextmanager
        def warm_producer(self, env, evidence, deadline):
            # The inherited stock migrator and runner remain untouched.
            self.broker_file_stamp = arm_pinned_file(self.probe_broker, self.probe_sha)
            try:
                with base_subject.warm_producer(self, env, evidence, deadline) as enqueue:
                    yield enqueue
            finally:
                self.broker_launch = None

        def capture_runners(self, processes, logs):
            measured = copy(self)
            measured.broker = self.probe_broker
            measured.broker_sha = self.probe_sha
            measured.broker_manifest_sha = self.probe_manifest_sha
            subject.capture_runners(measured, processes, logs)
            path = self.cluster.evidence / "split-executables.json"
            receipt = json.loads(path.read_text())
            receipt["migrator"] = str(self.broker)
            path.write_text(json.dumps(receipt, sort_keys=True) + "\n")

    from identity import pinned_file as arm_pinned_file
    arm.Subject = ObservedSubject
    arm.Cluster = ObservedCluster
    arm.database_settings = settings
