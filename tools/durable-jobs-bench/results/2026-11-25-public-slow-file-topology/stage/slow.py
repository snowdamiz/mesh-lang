"""Install the public FILE adapter on the frozen SLOW trial."""

from collections import Counter
import hashlib
import inspect
import json
import os
from pathlib import Path
import sys
from textwrap import dedent
import types

HERE = Path(__file__).resolve().parent
INPUT = "42bb9e2be94c17b36110081430faf2647a04f3a3e0f2decf00e7c216fed462fd"


def load_named(name):
    # The matrix directory is traversable but cannot be listed by nobody.
    path = HERE / (name + ".py")
    module = types.ModuleType(name)
    module.__file__ = str(path)
    sys.modules[name] = module
    exec(compile(path.read_bytes(), str(path), "exec"), vars(module))
    return module


load_named("identity")
public = load_named("backend")
socket_map = load_named("socket_map")
arm = public.arm


def replace(text, before, after):
    if text.count(before) != 1:
        raise ValueError(f"frozen source anchor changed: {before}")
    return text.replace(before, after)


def install_function(original, before, after, **scope):
    source = replace(dedent(inspect.getsource(original)), before, after)
    namespace = dict(original.__globals__, **scope)
    exec(compile(source, original.__code__.co_filename, "exec"), namespace)
    return namespace[original.__name__]


def normalized_files(scratch, batches):
    paths, digests = [], []
    for batch in batches:
        path = scratch / f"{batch['request_id']}.json"
        data = (json.dumps(batch, indent=2, sort_keys=True, allow_nan=False) + "\n").encode()
        with path.open("xb") as stream:
            stream.write(data)
        paths.append(path)
        digests.append(hashlib.sha256(data).hexdigest())
    return paths, hashlib.sha256("".join(digests).encode()).hexdigest()


def install():
    public.install()
    public.install_broker_observer()
    # inspect reads the unpatched disk source. Reapply the two frozen
    # regression edits before changing only the fixture hash boundary.
    original = arm.h.run_trial
    source = dedent(inspect.getsource(original))
    for before, after in (
        ("if sparse else lambda path, log: subject.enqueue_receipt(env, path, log))",
         "if subject.shared_broker else lambda path, log: subject.enqueue_receipt(env, path, log))"),
        ('            wait_for(lambda: runner_ready(processes, logs), deadline, "runner readiness")\n',
         '            wait_for(lambda: runner_ready(processes, logs), deadline, "runner readiness")\n'
         '            subject.capture_runners(processes, logs)\n'),
        ('    paths = batch_files(scratch, batches)\n'
         '    case = hashlib.sha256("".join(digest(path) for path in paths).encode()).hexdigest()',
         '    paths, case = _normalized_files(scratch, batches)'),
    ):
        source = replace(source, before, after)
    scope = dict(original.__globals__, _normalized_files=normalized_files)
    exec(compile(source, original.__code__.co_filename, "exec"), scope)
    arm.h.run_trial = scope["run_trial"]
    arm.Cluster.audit_tls = install_function(
        arm.Cluster.audit_tls, "'backend', a.pid, 'tls'",
        "'backend', a.pid, 'client_port', a.client_port, 'tls'",
    )
    arm.Cluster, arm.Subject = Cluster, Subject


class Cluster(public.Cluster):
    def reconcile_trial(self, engine, namespace, enqueued_ids, handler_attempts, commits):
        result = super().reconcile_trial(engine, namespace, enqueued_ids, handler_attempts, commits)
        rows = json.loads(self.sql(engine, """
            SELECT json_agg(json_build_object('query', query, 'calls', calls, 'rows', rows,
                'wal_bytes', wal_bytes, 'execution_ms', total_exec_time) ORDER BY query)
            FROM pg_stat_statements
            WHERE userid = 'parity_producer'::regrole
              AND dbid = (SELECT oid FROM pg_database WHERE datname=current_database())
              AND (query LIKE 'SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(%'
                   OR query LIKE 'SELECT mesh_jobs.enqueue(%')
        """))
        assert [(row["calls"], row["rows"]) for row in rows] == [(8, 8), (8, 504)]
        assert all(row["wal_bytes"] > 0 for row in rows)
        capacity = int(self.sql(engine, "SELECT concurrency FROM mesh_jobs.queues WHERE name = 'default'"))
        assert capacity == self.slots == 64
        arm.h.write_json(self.evidence / "slow-transfer.json", dict(
            producer_statements=rows, queue_capacity=capacity, session_mode=os.environ["PRODUCT_SESSION_MODE"],
            slots_per_runner=64 // self.runners, warm_apps_by_runner=self.warm_apps_by_runner,
        ))
        return result


class Subject(public.Subject):
    def capture_runners(self, processes, logs):
        super().capture_runners(processes, logs)
        tls = json.loads((self.cluster.evidence / "postgres-tls-sessions.json").read_text())
        mapping = socket_map.runner_backend_map([child.pid for child in processes],
                                                self.cluster.port, tls)
        assert all(len(sessions) == self.cluster.lanes for sessions in mapping.values())
        backends = [session["backend"] for sessions in mapping.values() for session in sessions]
        assert len(backends) == len(set(backends)) == 8
        counts = Counter(session["role"] for session in tls)
        assert counts == dict(compiled_caller=8, parity_subject=4 * len(processes), parity_producer=1)
        self.cluster.warm_apps_by_runner = mapping


def main():
    assert os.geteuid() != 0 and sys.platform == "linux"
    assert sys.argv[1:3] == ["mesh", "slow"]
    assert os.environ["PRODUCT_SESSION_MODE"] in ("inline", "file")
    topology = os.environ["PRODUCT_TOPOLOGY"]
    assert topology in ("stock", "two-runner")
    assert os.environ["PRODUCT_STORAGE"] == "default"
    assert arm.PROFILES["slow"] == (1, 64, 8, 68, 2, INPUT)
    runners = 1 if topology == "stock" else 2
    arm.PROFILES["slow"] = (runners, 64, 8 // runners, 68, 2, INPUT)
    assert arm.h.allocate_slots(runners, 64) == [64 // runners] * runners
    install()
    os.environ.update(TRUSTED_ADMISSION_POLICY="set_based",
                      MESH_JOBS_SET_BASED_ENQUEUE="1", PRODUCT_CAPTURE_IDENTITY="1")
    artifact = Path(os.environ["NATIVE_JOBS_ARTIFACTS"])
    manifest = artifact / "mesh.json"
    assert arm.h.digest(manifest) == os.environ["PRODUCT_MANIFEST_SHA256"]
    build = json.loads(manifest.read_text())
    assert build["files"]["mesh"] == os.environ["PRODUCT_ELF_SHA256"]
    if os.environ.get("PUBLIC_SLOW_PREFLIGHT") == "1":
        import tempfile
        from unittest.mock import patch

        class ReachedFrozenParent(Exception):
            pass

        with tempfile.TemporaryDirectory(dir=Path(sys.argv[3])) as scratch:
            scratch = Path(scratch)
            settings = json.loads(arm.h.CONFIG.read_text())
            batches = arm.h.inputs(settings["workloads"]["slow"], 64, settings["seed"])
            paths, case = normalized_files(scratch, batches)
            assert case == INPUT
            original = scratch / "original"
            original.mkdir()
            reference, = arm.h.batch_files(original, batches)
            assert paths[0].read_bytes() == reference.read_bytes()
            # Exercise the repaired super() edge in the real closure without
            # entering PostgreSQL or running any fixture workload.
            frozen = public.Cluster.__bases__[0]
            instance = object.__new__(Cluster)
            instance.evidence = scratch
            instance.sql = lambda *_: "{}"
            with patch.object(arm.regression.Cluster, "reconcile_trial", side_effect=ReachedFrozenParent):
                try:
                    frozen.reconcile_trial(instance, "mesh", "parity_0123456789abcdef", {}, {}, {})
                except ReachedFrozenParent:
                    pass
                else:
                    raise AssertionError("the frozen reconcile parent was not reached")
        assert not os.access(HERE, os.R_OK)
        print(json.dumps(dict(uid=os.geteuid(), stage_listable=False, fixture=case,
                              profile=arm.PROFILES["slow"], mode=os.environ["PRODUCT_SESSION_MODE"],
                              class_cell=True, installed=True), sort_keys=True))
        return
    assert sorted(os.sched_getaffinity(0)) == [0, 1]
    arm.main()


if __name__ == "__main__":
    main()
