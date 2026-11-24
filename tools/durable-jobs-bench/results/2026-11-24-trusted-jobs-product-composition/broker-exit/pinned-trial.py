"""Run one frozen regression pair with one producer/observer connection per arm."""

from contextlib import contextmanager
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import traceback

import oban_runtime
import proof_transport

PROOF = Path("/proof")
spec = importlib.util.spec_from_file_location("harness", PROOF / "jobs-parity.py")
h = importlib.util.module_from_spec(spec)
source = Path(spec.origin).read_text()
anchor = "if sparse else lambda path, log: subject.enqueue_receipt(env, path, log))"
assert source.count(anchor) == 1
source = source.replace(anchor, "if subject.shared_broker else lambda path, log: subject.enqueue_receipt(env, path, log))")
ready = '            wait_for(lambda: runner_ready(processes, logs), deadline, "runner readiness")\n'
assert source.count(ready) == 1
source = source.replace(ready, ready + "            subject.capture_runners(processes, logs)\n")
exec(compile(source, spec.origin, "exec"), h.__dict__)
h.CONFIG = PROOF / "jobs-parity-profiles.json"

# runners, slots, app sessions per runner, physical Postgres cap, guest GiB, input digest.
PROFILES = {
    "noop": (8, 64, 4, 96, 2, "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"),
    "slow": (1, 64, 8, 68, 2, "42bb9e2be94c17b36110081430faf2647a04f3a3e0f2decf00e7c216fed462fd"),
    "sparse": (1, 1, 1, 5, 2, "20225faf6274696987b2192136f344d69be243f0a21dc9afad71e714c6ee08c5"),
    "short": (1, 16, 8, 20, 2, "ac0bcbf2465ea6d6807e8bfcdcfc3b1dacfa18b775e3f4c074257d66bfbba9d1"),
    "large": (8, 64, 4, 96, 4, "9d7f130f2cc690487582f73c9bea5fbb1f2661f8e416beb2f1574a6f868cc5c1"),
}


def process_table():
    rows = {}
    for path in Path("/proc").iterdir():
        if not path.name.isdecimal():
            continue
        try:
            fields = (path / "stat").read_text().rsplit(")", 1)[1].split()
        except (FileNotFoundError, ProcessLookupError):
            continue
        if fields[0] != "Z":
            rows[int(path.name)] = (int(fields[1]), int(fields[21]) * os.sysconf("SC_PAGE_SIZE") // 1024,
                                    fields[19])
    return rows


h.process_table = process_table


class Cluster(h.Cluster):
    previous_database = None
    broker = None

    def sql(self, database, query):
        if self.broker:
            scalar = "SELECT (" + query.strip().rstrip(";") + ")::text AS value"
            return self.broker({"verb": "query", "sql": scalar})
        return super().sql(database, query)

    def configure_tls(self):
        super().configure_tls()
        with (self.data / "postgresql.conf").open("a") as stream:
            stream.write(f"max_connections = {self.connection_cap}\n"
                         "superuser_reserved_connections = 0\nreserved_connections = 0\n"
                         "fsync = on\nfull_page_writes = on\n")

    def prepare_database(self, engine):
        if self.previous_database and self.previous_database != engine:
            self.sql("postgres", f"DROP DATABASE {self.previous_database}")
        self.previous_database = engine
        super().prepare_database(engine)
        self.sql(engine, "CREATE EXTENSION pg_stat_statements")
        limit = self.runners * 4 if engine == "mesh" else self.slots + self.runners * 2
        self.sql("postgres", f"""
            GRANT pg_read_all_stats TO parity_producer;
            ALTER ROLE parity_subject CONNECTION LIMIT {limit};
            ALTER ROLE parity_producer CONNECTION LIMIT 1;
        """)
        if engine == "mesh":
            self.sql("postgres", f"""
              DO $$ BEGIN
                IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'compiled_definer') THEN
                  CREATE ROLE compiled_definer NOLOGIN;
                  CREATE ROLE compiled_caller LOGIN;
                END IF;
              END $$;
              ALTER ROLE compiled_caller CONNECTION LIMIT {self.lanes * self.runners}
                PASSWORD '{self.password}';
            """)
            self.sql(engine, """
              REVOKE CONNECT, TEMPORARY ON DATABASE mesh FROM PUBLIC;
              GRANT CONNECT ON DATABASE mesh TO parity_subject, parity_producer, compiled_caller;
            """)

    def reconcile_trial(self, engine, namespace, enqueued_ids, handler_attempts, commits):
        result = super().reconcile_trial(engine, namespace, enqueued_ids, handler_attempts, commits)
        relation, scope = h.job_relation(engine, namespace)
        if engine == "mesh":
            scope = "j." + scope
        witness = json.loads(self.sql(engine, f"""
          SELECT json_build_object(
            'jobs', count(*),
            'worker_pids', count(DISTINCT worker_pid),
            'ordered', count(*) FILTER (WHERE a.xmin <> e.xmin
               AND pg_xact_commit_timestamp(a.xmin) < pg_xact_commit_timestamp(e.xmin)
               AND pg_xact_commit_timestamp(e.xmin) <= pg_xact_commit_timestamp(j.xmin)),
            'minimum_hold_ms', min(extract(epoch FROM
                (pg_xact_commit_timestamp(e.xmin) - pg_xact_commit_timestamp(a.xmin))) * 1000))
          FROM {relation} j
          JOIN public.mesh_jobs_parity_attempts a ON (a.job_key = j.args->>'key')
          JOIN public.mesh_jobs_parity_effects e ON
            (e.engine, e.namespace, e.job_key) = (a.engine, a.namespace, a.job_key)
          WHERE a.engine = '{engine}' AND a.namespace = '{namespace}' AND {scope}
        """))
        assert witness["ordered"] == witness["jobs"] == len(enqueued_ids), witness
        if engine == "mesh" and not getattr(self, "trusted_application", False):
            assert witness["worker_pids"] == witness["jobs"], witness
        else:
            assert 0 < witness["worker_pids"] <= self.runners, witness
        assert witness["minimum_hold_ms"] >= self.hold_ms - 1, witness
        h.write_json(self.evidence / "ordered-writes.json", witness)
        return result


class Subject(h.Subject):
    shared_broker = True

    def capture_oban(self, role, pid, log):
        table = process_table()
        report = oban_runtime.report_from(log.read_text())
        return oban_runtime.live_report(report, self.cluster.proof_config, role, pid, table,
                                        h.process_tree(table, {pid}))

    def capture_runners(self, processes, logs):
        if self.engine == "oban":
            reports = [self.capture_oban("run", process.pid, log)
                       for process, log in zip(processes, logs, strict=True)]
            h.write_json(self.cluster.evidence / "oban-emulator.json", reports)

    @contextmanager
    def warm_producer(self, env, evidence, deadline):
        cluster = self.cluster
        env = dict(env, JOBS_PARITY_DB_URL=cluster.url(self.engine, "parity_producer"))
        output = evidence / "broker-output.log"
        errors = evidence / "broker-errors.log"
        lock = threading.Lock()

        with output.open("wb") as sink, errors.open("wb") as stderr, subprocess.Popen(
            [*self.command, "session"], env=env, stdin=subprocess.PIPE, stdout=sink, stderr=stderr,
        ) as broker, output.open("rb") as reader:
            def poll_line():
                offset = reader.tell()
                line = reader.readline()
                if line.endswith(b"\n"):
                    return line.decode().rstrip("\n") or "\n"
                reader.seek(offset)
                if broker.poll() is not None:
                    raise RuntimeError(errors.read_text())
                return None

            def reply():
                return h.wait_for(poll_line, min(deadline, h.time.monotonic() + 15),
                                  "shared producer/observer response", 0.001)

            def request(command):
                with lock:
                    broker.stdin.write(json.dumps(command, separators=(",", ":")).encode() + b"\n")
                    broker.stdin.flush()
                    return reply()

            def enqueue(path, log):
                batch = json.loads(path.read_text()) if self.engine == "mesh" else str(path)
                response = request({"verb": "enqueue", "batch": batch})
                log.write_text(response + "\n")
                return self.read_receipt(response, log)

            try:
                assert reply() == "SESSION_READY"
                if self.engine == "oban":
                    h.write_json(evidence / "oban-broker-emulator.json",
                                 self.capture_oban("session", broker.pid, errors))
                cluster.broker = request
                identity = json.loads(cluster.sql(self.engine, """
                    SELECT json_build_object(
                      'user', current_user,
                      'tls', (SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()),
                      'max_connections', current_setting('max_connections')::integer,
                      'database_connection_limit', (SELECT datconnlimit FROM pg_database
                                                      WHERE datname = current_database()),
                      'reserve', current_setting('reserved_connections')::integer,
                      'superuser_reserve', current_setting('superuser_reserved_connections')::integer,
                      'durable', current_setting('synchronous_commit') = 'on'
                                 AND current_setting('fsync') = 'on'
                                 AND current_setting('full_page_writes') = 'on',
                      'commit_timestamps', current_setting('track_commit_timestamp') = 'on')
                """))
                assert identity == dict(user="parity_producer", tls=True, durable=True, commit_timestamps=True,
                                        max_connections=cluster.connection_cap,
                                        database_connection_limit=cluster.connection_cap,
                                        reserve=0, superuser_reserve=0), identity
                h.write_json(evidence / "database.json", identity)
                yield enqueue
                broker.stdin.write(b"stop\n")
                broker.stdin.flush()
                assert json.loads(reply()) == {}
                assert broker.wait(timeout=5) == 0
            finally:
                cluster.broker = None
                if broker.poll() is None:
                    broker.kill()

    def call(self, action, env, log):
        result = super().call(action, env, log)
        if action == "migrate" and self.engine == "mesh":
            if getattr(self.cluster, "trusted_application", False):
                namespace = env["JOBS_PARITY_NAMESPACE"]
                h.job_relation("mesh", namespace)
                sql = (PROOF / "trusted-application.sql").read_text()
                self.cluster.sql("mesh", sql.replace("@NAMESPACE@", namespace))
            else:
                self.cluster.sql("mesh", "SET ROLE parity_subject;\n" + (PROOF / "fence.sql").read_text())
                self.cluster.sql("mesh", (PROOF / "application.sql").read_text())
        return result


def oban_environment(env, config):
    return dict(env, ERL_FLAGS=config["oban_erl_flags"],
                JOBS_PARITY_OBAN_UNSET_INETRC="0" if config["oban_erl_inetrc"] else "1")


def measure(profile, arm, warmup, root, evidence, cluster, settings, workload, batches):
    label = f"warmup-{arm}" if warmup else arm
    print(f"FROZEN_DIAGNOSTIC_ARM {profile} {'warmup' if warmup else 'measured'} {arm}", flush=True)
    scratch, out = root / label, evidence / label
    scratch.mkdir()
    out.mkdir()
    cluster.evidence = out
    env = dict(cluster.env, TMPDIR=str(scratch),
               MESH_SCHEDULER_MIN_WORKERS="2", MESH_SCHEDULER_MAX_WORKERS="2")
    if arm == "mesh":
        env.update(JOBS_PARITY_MESH_BATCH="1", JOBS_PARITY_MESH_LISTEN="1",
                   MESH_EFFECT_LANES=str(cluster.lanes), MESH_EFFECT_APP_URL=cluster.url("mesh", "compiled_caller"),
                   MESH_DIAGNOSTIC_JOB_DISABLE_THP="1")
    if arm == "oban":
        env = oban_environment(env, cluster.proof_config)
    command = PROOF / ("oban" if arm == "oban" else "mesh-current")
    subject = Subject(arm, [str(command)], PROOF, cluster, env)
    result = h.run_trial(subject, h.allocate_slots(cluster.runners, cluster.slots), workload,
                         batches, scratch, out, settings, 0)
    assert result["input_sha256"] == PROFILES[profile][-1], result
    assert result["completed"] == sum(len(batch["jobs"]) for batch in batches), result
    assert result["peak_executing"] <= cluster.slots, result
    assert result["peak_connections"] <= cluster.connection_cap, result
    if arm == "mesh":
        assert result["application_sql_calls_per_job"] > 0, result
    else:
        assert result["application_sql_calls_per_job"] == 0, result
    if profile == "sparse":
        assert result["peak_connections"] == (5 if arm == "mesh" else 4), result
    h.write_json(out / "metrics.json", result)
    return result


def run(config, root, evidence):
    assert os.uname().machine == "aarch64" and os.getuid() == 65534 and os.cpu_count() == 2
    profile = config["profile"]
    runners, slots, lanes, cap, memory, _ = PROFILES[profile]
    mem_kib = int(Path("/proc/meminfo").read_text().splitlines()[0].split()[1])
    assert (memory - 0.5) * 1024**2 < mem_kib <= memory * 1024**2, mem_kib
    settings = json.loads(h.CONFIG.read_text())
    assert config["oban_erl_flags"] == settings["oban_erl_flags"], config
    workload = settings["workloads"][profile]
    batches = h.inputs(workload, slots, settings["seed"])
    env = oban_environment(os.environ, config)
    preflight = h.checked([str(PROOF / "oban"), "preflight"], log=evidence / "oban-preflight.log", env=env)
    report = oban_runtime.report_from(preflight)
    oban_runtime.verify_report(report, config, "preflight")
    h.write_json(evidence / "oban-preflight.json", report)
    versions = h.checked([str(PROOF / "oban"), "versions"], log=evidence / "oban-versions.log", env=env)
    assert json.loads(versions.splitlines()[-1]) == {"oban": "2.24.2", "erlang": "29.1.1", "elixir": "1.20.4"}

    scratch, out = root / "cluster", evidence / "cluster"
    scratch.mkdir()
    out.mkdir()
    cluster = Cluster(scratch, out, cap, "17.11")
    cluster.proof_config = config
    cluster.runners, cluster.slots, cluster.lanes, cluster.hold_ms = runners, slots, lanes, workload["hold_ms"]
    try:
        cluster.start()
        for warmup in range(int(config["full_input_warmup"]), -1, -1):
            results = [measure(profile, arm, warmup, root, evidence, cluster, settings, workload, batches)
                       for arm in config["arms"]]
            h.write_json(evidence / ("warmups.json" if warmup else "metrics.json"), results)
    finally:
        cluster.stop()


def main():
    config = json.loads((PROOF / "config.json").read_text())
    with tempfile.TemporaryDirectory(dir="/tmp", prefix="queue-perf-") as temp:
        root = Path(temp)
        evidence = root / "evidence"
        evidence.mkdir()
        h.write_json(evidence / "config.json", config)
        status = 0
        try:
            for name, checksum in config["hashes"].items():
                assert h.digest(Path("/") / name) == checksum, name
            run(config, root, evidence)
        except Exception:
            (evidence / "failure.log").write_text(traceback.format_exc())
            status = 1
        proof_transport.emit(evidence, root / "proof.tar.gz", None, sys.stdout.buffer)
        return status


if __name__ == "__main__":
    sys.exit(main())
