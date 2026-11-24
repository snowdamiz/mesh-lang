"""Apply the one slow topology/cap override to a pinned native launcher process."""

from collections import Counter
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sys
import time

CAP = 71
INPUT = "42bb9e2be94c17b36110081430faf2647a04f3a3e0f2decf00e7c216fed462fd"
LAUNCHERS = {
    "mesh": "f01706da43f294bb1130b95674adf30afe53c9349084cda5ef8cc8f2abdb7554",
    "oban": "95766ac87a9f04dfdd253af52a04d83438d970da7d01c6cdadfea682cebb473e",
    "go": "e97ede452fb9df9c62fc9b12c914dbb2b5c92b2cd64165b7d11ac5e2dcc5d030",
    "rust": "05c94874455c7c48e0ca6e9204b412aa67d41096b6cf314c0d4a9f3a8fdaf0f7",
}
SESSIONS = """
SELECT coalesce(json_agg(json_build_object(
    'role', a.usename, 'backend', a.pid, 'tls', coalesce(s.ssl, false),
    'client_port', a.client_port, 'state', a.state,
    'query_us', (extract(epoch FROM a.query_start) * 1000000)::bigint,
    'query', a.query) ORDER BY a.pid), '[]')
FROM pg_stat_activity a LEFT JOIN pg_stat_ssl s USING (pid)
WHERE a.datname = current_database()
  AND a.usename IN ('parity_subject', 'parity_producer', 'compiled_caller')
"""
STATEMENTS = """
SELECT coalesce(json_agg(to_jsonb(w) ORDER BY w.query), '[]') FROM (
    SELECT rolname AS role, query, calls, rows, total_exec_time, wal_bytes
    FROM pg_stat_statements JOIN pg_roles ON userid = pg_roles.oid
    WHERE dbid = (SELECT oid FROM pg_database WHERE datname = CURRENT_DATABASE())
      AND rolname IN ('parity_producer', 'parity_subject', 'compiled_caller')
) w
"""
CAPS = """
SELECT json_build_object(
    'sql_queues', (SELECT json_agg(to_jsonb(q)) FROM mesh_jobs.queues q),
    'role_caps', (SELECT json_object_agg(rolname, rolconnlimit) FROM pg_roles
                  WHERE rolname IN ('parity_subject', 'parity_producer', 'compiled_caller')))
"""


def write(path, content):
    with path.open("x") as stream:
        json.dump(content, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")


def load(engine, path):
    if hashlib.sha256(path.read_bytes()).hexdigest() != LAUNCHERS[engine]:
        raise ValueError(f"stock {engine} launcher changed")
    sys.path.insert(0, str(path.parent))
    spec = importlib.util.spec_from_file_location("slow_equal_stock", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def instrument(base, dbname, out):
    class Cluster(base):
        def start(self):
            super().start()
            pid = int((self.data / "postmaster.pid").read_text().splitlines()[0])
            write(out / "postgres-affinity.json", dict(pid=pid, cpus=sorted(os.sched_getaffinity(pid))))

        def configure_tls(self):
            super().configure_tls()
            with (self.data / "postgresql.conf").open("a") as stream:
                stream.write("log_connections = on\nlog_disconnections = on\nlog_timezone = 'UTC'\n"
                             "log_line_prefix = '%m [%p] %u %d %a | '\n")

        def stop(self):
            log = self.evidence / "postgres.log"
            try:
                if log.exists() and (self.data / "postmaster.pid").exists():
                    # The stock trial has closed its runners and producer. This
                    # transient admin seat is outside the measured arm window.
                    result = self.sql("postgres", STATEMENTS.replace("CURRENT_DATABASE()", f"'{dbname}'"))
                    data = self.sql("postgres", "SELECT json_build_object('cap', current_setting('max_connections')::int, "
                                    "'dealloc', (SELECT dealloc FROM pg_stat_statements_info), 'sessions', "
                                    "(SELECT coalesce(json_object_agg(usename, n), '{}') FROM "
                                    "(SELECT usename, count(*) AS n FROM pg_stat_activity "
                                    "WHERE backend_type = 'client backend' GROUP BY usename) roles))")
                    write(out / "sql-lifecycle.json", dict(statements=json.loads(result), post_arm=json.loads(data)))
            finally:
                try:
                    super().stop()
                finally:
                    if log.exists():
                        (out / "physical-postgres.log").write_text(
                            log.read_text(errors="replace").replace(self.password, "<redacted>"))

    return Cluster


def mesh_or_oban(engine, adapter, out):
    if adapter.PROFILES["slow"] != (1, 64, 8, 68, 2, INPUT):
        raise ValueError("unexpected stock native slow profile")
    adapter.PROFILES = {"slow": (2, 64, 4, CAP, 2, INPUT)}
    base = instrument(adapter.Cluster, engine, out)
    roles = {"parity_subject": 8, "parity_producer": 1, "compiled_caller": 8}
    if engine == "oban":
        roles = {"parity_subject": 68, "parity_producer": 1}
    else:
        os.environ["MESH_JOBS_SET_BASED_ENQUEUE"] = "1"

    class Cluster(base):
        counter_samples = 0

        def sessions(self, name):
            return json.loads(self.sql(name, SESSIONS))

        def audit_tls(self, name):
            def warm():
                rows = self.sessions(name)
                return len(rows) == sum(roles.values()) and Counter(
                    row["role"] for row in rows if row["tls"]) == roles

            adapter.h.wait_for(warm, time.monotonic() + 15, "exact warm slow roles", 0.05)
            super().audit_tls(name)
            if name == "mesh":
                write(out / "sql-capacity.json", json.loads(self.sql(name, CAPS)))

        def calls(self, name):
            self.counter_samples += 1
            if self.counter_samples == 2:
                write(out / "sessions-before.json", self.sessions(name))
                write(out / "statements-before.json", json.loads(self.sql(name, STATEMENTS)))
            counters = super().calls(name)
            write(out / f"counters-{self.counter_samples}.json", counters)
            if self.counter_samples == 3:
                write(out / "sessions-end.json", self.sessions(name))
                write(out / "statements-end.json", json.loads(self.sql(name, STATEMENTS)))
                write(out / "physical-end.json", dict(physical_clients=len(self.sessions(name)), by_role=roles))
            return counters

        def observe(self, name, namespace):
            result = super().observe(name, namespace)
            # Whole-cluster physical activity is independently accounted for
            # by connection logs. This is the base measured-role count.
            result["physical_clients"] = result["connections"]
            return result

    class Subject(adapter.Subject):
        roster = {}

        def run(self, env, index, log):
            slots = int(env["JOBS_PARITY_SLOTS"])
            if slots != 32:
                raise ValueError(f"unexpected runner slot count {slots}")
            process = super().run(env, index, log)
            self.roster[process.pid] = dict(index=index, slots=slots)
            return process

        def capture_runners(self, processes, logs):
            super().capture_runners(processes, logs)
            if {process.pid for process in processes} != set(self.roster) or len(self.roster) != 2:
                raise ValueError("unexpected native runner roster")
            write(out / "runner-topology.json", self.roster)
            if self.engine == "mesh":
                write(out / "topology.json", dict(label="two", runners=2, slots_per_runner=32,
                      app_sessions_per_runner=4, scheduler_min=int(self.env["MESH_SCHEDULER_MIN_WORKERS"]),
                      scheduler_max=int(self.env["MESH_SCHEDULER_MAX_WORKERS"]),
                      runner_pids_by_index=[process.pid for process in processes], wire=False))

    adapter.Cluster, adapter.Subject = Cluster, Subject


def river_or_apalis(engine, adapter, out):
    import verify
    if verify.PROFILES["slow"] != (1, 64, 68, INPUT, True):
        raise ValueError("unexpected stock matrix slow profile")
    # Process-local override. The stock verifier receives the actual ledger.
    verify.PROFILES["slow"] = (2, 64, CAP, INPUT, True)
    adapter.Cluster = instrument(adapter.Cluster, "river" if engine == "go" else "apalis", out)
    if engine != "rust":
        return None
    original = adapter.parity.RunnerProcess
    roster = {}

    def launch(argv, **kwargs):
        if argv[1] != "worker" or argv[2] not in ("runner-0", "runner-1") or argv[3] != "32":
            raise ValueError("unexpected Apalis runner argv")
        process = original(argv, **kwargs)
        roster[argv[2]] = dict(pid=process.pid, slots=int(argv[3]))
        return process

    adapter.parity.RunnerProcess = launch
    return roster


def main():
    engine, launcher, *args = sys.argv[1:]
    if (sys.platform != "linux" or os.getuid() == 0 or sorted(os.sched_getaffinity(0)) != [0, 1]
            or engine not in LAUNCHERS or len(args) < 2 or args[-2] != "slow"
            or any(flag in args for flag in ("--smoke", "--local-smoke"))):
        raise ValueError("requires a pinned unprivileged native slow arm on CPUs 0,1")
    path, out = Path(launcher).resolve(strict=True), Path(args[-1]).resolve(strict=True)
    if any(out.iterdir()):
        raise ValueError("expected a fresh output directory")
    adapter = load(engine, path)
    roster = None
    if engine in ("mesh", "oban"):
        mesh_or_oban(engine, adapter, out)
    else:
        roster = river_or_apalis(engine, adapter, out)
    sys.argv = [str(path), *args]
    adapter.main()
    if roster is not None:
        if set(roster) != {"runner-0", "runner-1"} or len({r["pid"] for r in roster.values()}) != 2:
            raise ValueError("unexpected Apalis runner roster")
        write(out / "runner-topology.json", roster)


if __name__ == "__main__":
    main()
