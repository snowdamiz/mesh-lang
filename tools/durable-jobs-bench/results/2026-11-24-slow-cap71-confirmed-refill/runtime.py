"""Build-free slow-2x32 adapter for the two retained product ELFs."""
from collections import Counter
from contextlib import contextmanager
from copy import copy
import inspect
import json
import os
from pathlib import Path
import re
import sys
from textwrap import dedent
import time

sys.path.insert(0, str(Path(__file__).resolve().parent / "deps"))
import backend
import cap71
from identity import observe, pinned_file, process_stamp

arm = backend.arm
CAP = cap71.CAP
INPUT = cap71.INPUT
ROLE_CAPS = {"parity_subject": 8, "compiled_caller": 8, "parity_producer": 1}
REFILL = "mesh_jobs.complete_and_refill_v1(text,text,jsonb,bigint,integer,integer,uuid[],uuid[])"
BASE_SUBJECT = arm.Subject


def query_dict(db, sql):
    return json.loads(db.sql("mesh", sql))


def capacity(db):
    data = query_dict(db, cap71.CAPS)
    rows = data["sql_queues"]
    row, = [row for row in rows if row["name"] != ""]
    assert all(r["namespace"] == row["namespace"] and r["state"] == "active" for r in rows), rows
    assert row["name"] == "default" and row["concurrency"] == 64 and row["state"] == "active", row
    assert data["role_caps"] == ROLE_CAPS, data
    return data


def scope_refill(db):
    db.sql("mesh", f"GRANT EXECUTE ON FUNCTION {REFILL} TO parity_subject;")
    assert db.sql("mesh", f"""
        SELECT has_function_privilege('parity_subject', '{REFILL}', 'EXECUTE')
          AND NOT has_function_privilege('parity_producer', '{REFILL}', 'EXECUTE')
          AND NOT has_function_privilege('compiled_caller', '{REFILL}', 'EXECUTE')
    """) == "t"


def make_cluster(out):
    Base = cap71.instrument(backend.Cluster, "mesh", out)

    class Cluster(Base):
        def audit_tls(self, engine):
            def ready():
                rows = query_dict(self, cap71.SESSIONS)
                return Counter(r["role"] for r in rows if r["tls"]) == ROLE_CAPS and len(rows) == 17

            arm.h.wait_for(ready, time.monotonic() + 15, "eight app and eight queue TLS sessions", 0.05)
            super().audit_tls(engine)
            cap71.write(out / "capacity-before.json", capacity(self))

        def reconcile_trial(self, engine, namespace, enqueued_ids, handler_attempts, commits):
            result = super().reconcile_trial(engine, namespace, enqueued_ids, handler_attempts, commits)
            cap71.write(out / "capacity-after.json", capacity(self))
            assert re.fullmatch(r"parity_[0-9a-f]{16}", namespace)
            storage = query_dict(self, f"""
                SELECT json_build_object('rows', count(*),
                    'compressed', count(pg_column_compression(args)),
                    'min_bytes', min(octet_length(args->>'payload')),
                    'max_bytes', max(octet_length(args->>'payload')))
                FROM mesh_jobs.jobs WHERE namespace = '{namespace}'
            """)
            cap71.write(out / "canonical-storage.json", storage)
            runner_status = [arm.h.mesh_shutdown_report(out / f"runner-{i}.log") for i in range(2)]
            assert all(status["last_error"] is None for status in runner_status), runner_status
            trace = query_dict(self, f"""
                SELECT json_object_agg(id, attempted_at_ms)
                FROM mesh_jobs.jobs WHERE namespace = '{namespace}'
            """)
            cap71.write(out / "refill-ledger.json", dict(
                runner_status=runner_status, attempted_at_ms=trace))
            return result

    return Cluster


def app_socket_owners(pids, postgres_port, sessions):
    ports = {r["client_port"]: r["backend"] for r in sessions if r["role"] == "compiled_caller" and r["tls"]}
    peer = f"0100007F:{postgres_port:04X}"
    sockets = {}
    for line in Path("/proc/net/tcp").read_text().splitlines()[1:]:
        fields = line.split()
        if fields[2] == peer and fields[3] == "01":
            sockets[fields[9]] = int(fields[1].split(":")[1], 16)
    owners = {}
    for pid in pids:
        observed = set()
        for fd in Path(f"/proc/{pid}/fd").iterdir():
            try:
                link = os.readlink(fd)
            except FileNotFoundError:
                continue
            match = re.fullmatch(r"socket:\[(\d+)\]", link)
            if match and (port := sockets.get(match[1])) in ports:
                observed.add(ports[port])
        assert len(observed) == 4, (pid, observed)
        owners[pid] = sorted(observed)
    assert len({backend for backends in owners.values() for backend in backends}) == 8
    return owners


class Subject(backend.Subject):
    def call(self, action, env, log):
        result = super().call(action, env, log)
        if action == "migrate" and os.environ["SLOW_IMAGE"] == "candidate":
            scope_refill(self.cluster)
        return result

    @contextmanager
    def warm_producer(self, env, evidence, deadline):
        producer = Path(os.environ["SLOW_PRODUCER_PATH"])
        digest = os.environ["SLOW_PRODUCER_SHA"]
        stamp = pinned_file(producer, digest)
        owner = copy(self)
        owner.command = [str(producer)]
        with BASE_SUBJECT.warm_producer(owner, env, evidence, deadline) as enqueue:
            self.producer = owner.broker_process, producer, stamp, digest
            yield enqueue

    def run(self, env, index, log):
        assert int(env["JOBS_PARITY_SLOTS"]) == 32 and int(env["MESH_EFFECT_LANES"]) == 4
        assert "mesh_pool_hot_checkout" not in env["MESH_EFFECT_APP_URL"]
        env = dict(env, PRODUCT_FUSED_REFILL=os.environ["SLOW_FUSED"])
        return super().run(env, index, log)

    def capture_runners(self, processes, logs):
        BASE_SUBJECT.capture_runners(self, processes, logs)
        (broker, broker_start), producer, stamp, digest = self.producer
        cpus = sorted(os.sched_getaffinity(0))
        assert len(processes) == 2 and broker not in processes
        proof = [observe(broker, producer, "session", digest, stamp, broker_start, cpus)]
        proof.extend(observe(child, Path(self.command[0]), "run", self.executable_sha,
                             *child.product_stamp, cpus) for child in processes)
        out = self.cluster.evidence
        cap71.write(out / "application-sockets.json", app_socket_owners(
            [child.pid for child in processes], self.cluster.port, query_dict(self.cluster, cap71.SESSIONS)))
        cap71.write(out / "process-identity.json", proof)
        cap71.write(out / "runner-topology.json", {
            child.pid: {"index": index, "slots": 32} for index, child in enumerate(processes)
        })


def install_broker_observer():
    parent = arm.regression.Subject
    original = parent.warm_producer.__wrapped__
    source = dedent(inspect.getsource(original))
    anchor = '    ) as broker, output.open("rb") as reader:\n'
    eof = "            if broker.poll() is not None:\n                raise RuntimeError(errors.read_text())"
    assert source.count(anchor) == source.count(eof) == 1
    source = source.replace(anchor, anchor + '        if sys.platform == "linux":\n'
                            '            self.broker_process = (broker, process_stamp(broker))\n')
    # A fast, clean broker exit can fall between the first EOF and poll.
    # Re-read the response before concluding that the broker omitted it.
    source = source.replace(eof, """            if broker.poll() is not None:
                line = reader.readline()
                if line.endswith(b"\\n"):
                    return line.decode().rstrip("\\n") or "\\n"
                raise RuntimeError(errors.read_text())""")
    scope = dict(original.__globals__, process_stamp=process_stamp)
    exec(compile(source, original.__code__.co_filename, "exec"), scope)
    parent.warm_producer = scope["warm_producer"]


def install():
    assert arm.PROFILES["slow"] == (1, 64, 8, 68, 2, INPUT)
    arm.PROFILES["slow"] = (2, 64, 4, CAP, 2, INPUT)
    backend.install()
    install_broker_observer()


def main():
    assert sys.platform == "linux" and sys.argv[1:3] == ["mesh", "slow"]
    assert sorted(os.sched_getaffinity(0)) == [0, 1]
    assert os.environ["SLOW_IMAGE"] in {"public", "candidate"}
    assert os.environ["SLOW_FUSED"] in {"0", "1"}
    assert os.environ["SLOW_FUSED"] != "1" or os.environ["SLOW_IMAGE"] == "candidate"
    os.environ.update(PRODUCT_COMPOSITION="product-group", PRODUCT_STORAGE="default",
                      PRODUCT_TOPOLOGY="slow-equal-cap71", PRODUCT_CAPTURE_IDENTITY="1",
                      TRUSTED_ADMISSION_POLICY="success_cohort", MESH_JOBS_SET_BASED_ENQUEUE="1")
    manifest = Path(os.environ["NATIVE_JOBS_ARTIFACTS"]) / "mesh.json"
    assert arm.h.digest(manifest) == os.environ["PRODUCT_MANIFEST_SHA256"]
    assert json.loads(manifest.read_text())["files"]["mesh"] == os.environ["PRODUCT_ELF_SHA256"]
    install()
    arm.Cluster = make_cluster(Path(sys.argv[3]))
    arm.Subject = Subject
    arm.main()


if __name__ == "__main__":
    main()
