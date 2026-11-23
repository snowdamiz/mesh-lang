"""Run and reconcile a single native Mesh or Oban parity arm."""

import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import sys
import tempfile
import time
import traceback
import types

import ledger

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
PROOF = ROOT / "proof/jobs-parity/linux-regression"
PG_VERSION = "17.11"
VERSIONS = {"oban": "2.24.2", "erlang": "29.1.1", "elixir": "1.20.4"}
INETRC_SHA256 = "ea95be6f379d135ab873d2da91daeb4d6fcbb593e27db22914bf52aca8543a9c"
MESH_REVISION = "dbcb6dbc1de3a6887fc3757c2b91f7974d74ed17"


def regression_adapter():
    # Reuse the frozen broker, DB roles/caps, and trial loop. Only relocate
    # guest paths. Each replacement must match the pinned source exactly once.
    sys.path.insert(0, str(PROOF))
    path = PROOF / "trial.py"
    source = path.read_text()
    for before, after in (
        ('PROOF = Path("/proof")', f"PROOF = Path({str(PROOF)!r})"),
        ('PROOF / "jobs-parity.py"', f"Path({str(ROOT / 'scripts/jobs-parity.py')!r})"),
        ('PROOF / "jobs-parity-profiles.json"', f"Path({str(ROOT / 'scripts/jobs-parity-profiles.json')!r})"),
    ):
        if source.count(before) != 1:
            raise RuntimeError(f"frozen trial anchor changed: {before}")
        source = source.replace(before, after)
    module = types.ModuleType("native_regression")
    exec(compile(source, path, "exec"), module.__dict__)
    if platform.system() != "Linux":
        # Portable dev smoke uses the base harness sampler.
        native_h = module.h
        base = types.ModuleType("base_harness")
        base.__file__ = native_h.__file__
        exec(compile(Path(base.__file__).read_text(), base.__file__, "exec"), base.__dict__)
        native_h.process_table = base.process_table
        module.process_table = base.process_table
    return module


regression = regression_adapter()
h = regression.h
PROFILES = dict(regression.PROFILES, dense=regression.PROFILES["noop"])


class Cluster(regression.Cluster):
    def __init__(self, scratch, evidence, cap):
        super().__init__(scratch, evidence, cap, PG_VERSION)
        self.transactions = {}
        self.tls_roles = set()

    def audit_tls(self, engine):
        rows = json.loads(self.sql(engine, """
          SELECT coalesce(json_agg(json_build_object('role', a.usename,
                    'backend', a.pid, 'tls', coalesce(s.ssl, false))), '[]')
          FROM pg_stat_activity a LEFT JOIN pg_stat_ssl s USING (pid)
          WHERE a.datname = current_database()
            AND a.usename IN ('parity_subject', 'parity_producer', 'compiled_caller')
        """))
        if not rows or not all(row["tls"] for row in rows):
            raise RuntimeError("runner, application, or producer used a non-TLS PostgreSQL session")
        self.tls_roles = {row["role"] for row in rows}
        h.write_json(self.evidence / "postgres-tls-sessions.json", rows)
        roles = {"parity_subject", "parity_producer"}
        if engine == "mesh":
            roles.add("compiled_caller")
        if self.tls_roles != roles:
            raise RuntimeError(f"incomplete TLS session evidence: {self.tls_roles}")

    def reconcile_trial(self, engine, namespace, enqueued_ids, handler_attempts, commits):
        relation, scope = h.job_relation(engine, namespace)
        # Export complete application record counts even when reconciliation
        # rejects stray records and the shared harness cannot write raw.json.
        h.write_json(self.evidence / "application-records.json", json.loads(self.sql(engine, """
          SELECT json_build_object(
            'attempts', (SELECT coalesce(json_agg(row_to_json(a)), '[]') FROM (
              SELECT engine, namespace, job_key, effect_key, attempt, xmin::text AS xid
              FROM public.mesh_jobs_parity_attempts
            ) a),
            'effects', (SELECT coalesce(json_agg(row_to_json(e)), '[]') FROM (
              SELECT engine, namespace, job_key, effect_key, xmin::text AS xid
              FROM public.mesh_jobs_parity_effects
            ) e))
        """)))
        result = super().reconcile_trial(engine, namespace, enqueued_ids, handler_attempts, commits)
        if engine == "mesh":
            scope = "j." + scope
        self.receipts, self.jobs = ledger.collect(
            self, engine, namespace, self.transactions, handler_attempts, relation, scope
        )
        return result


class Subject(regression.Subject):
    def capture_runners(self, processes, logs):
        if sys.platform == "linux":
            super().capture_runners(processes, logs)
            pid = int((self.cluster.data / "postmaster.pid").read_text().splitlines()[0])
            expected = os.sched_getaffinity(0)
            masks = {child: sorted(os.sched_getaffinity(child))
                     for child in (pid, *(process.pid for process in processes))}
            if any(mask != sorted(expected) for mask in masks.values()):
                raise RuntimeError("PostgreSQL or runner CPU affinity differs from trial affinity")
            h.write_json(self.cluster.evidence / "process-affinity.json", masks)
        self.cluster.audit_tls(self.engine)

    @contextmanager
    def warm_producer(self, env, evidence, deadline):
        with super().warm_producer(env, evidence, deadline) as enqueue:
            def record_enqueue(path, log):
                submitted = time.time_ns() // 1000
                receipt, committed = enqueue(path, log)
                acknowledged = time.time_ns() // 1000
                self.cluster.transactions[path.stem] = {
                    "request_id": path.stem,
                    "submitted_us": submitted,
                    "acknowledged_us": acknowledged,
                    "xid": receipt["xid"],
                    "enqueued_ids": receipt["ids"],
                }
                return receipt, committed

            yield record_enqueue

    def capture_oban(self, role, pid, log):
        if sys.platform == "linux":
            return super().capture_oban(role, pid, log)
        return {"role": role, "pid": pid, "linux_procfs_available": False}


def host_snapshot():
    result = {
        "kernel": platform.release(), "system": platform.system(), "architecture": platform.machine(),
        "python": platform.python_version(), "host_logical_cpus": os.cpu_count(), "loadavg": os.getloadavg(),
    }
    if sys.platform == "linux":
        result["cpus"] = sorted(os.sched_getaffinity(0))
        for key, path in (("stat", "/proc/stat"), ("meminfo", "/proc/meminfo"),
                          ("cpu_pressure", "/proc/pressure/cpu"), ("io_pressure", "/proc/pressure/io")):
            if Path(path).exists():
                result[key] = Path(path).read_text()
    return result


def database_settings(cluster, engine):
    return json.loads(cluster.sql(engine, """
      SELECT json_build_object(
        'version', current_setting('server_version_num')::integer / 10000 || '.' ||
                   current_setting('server_version_num')::integer % 10000,
        'tls', (SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()),
        'fsync', current_setting('fsync')::boolean,
        'full_page_writes', current_setting('full_page_writes')::boolean,
        'synchronous_commit', current_setting('synchronous_commit') = 'on',
        'track_commit_timestamp', current_setting('track_commit_timestamp')::boolean,
        'max_connections', current_setting('max_connections')::int,
        'database_connection_limit', (SELECT datconnlimit FROM pg_database
                                      WHERE datname = current_database()),
        'reserved_connections', current_setting('reserved_connections')::int,
        'superuser_reserved_connections', current_setting('superuser_reserved_connections')::int)
    """))


def redact(evidence, password):
    secret = password.encode()
    for path in evidence.rglob("*"):
        if path.is_file() and not path.is_symlink():
            data = path.read_bytes()
            if secret in data:
                path.write_bytes(data.replace(secret, b"<redacted>"))


def failure(out, error, password=None):
    report = {"type": type(error).__name__, "stage": [
        [Path(frame.filename).name, frame.lineno, frame.name]
        for frame in traceback.extract_tb(error.__traceback__)
    ]}
    if password is not None:
        report["message"] = str(error).replace(password, "<redacted>")[:2000]
    h.write_json(out / "failure.json", report)


def run_arm(args, out, state, artifacts):
    engine, profile = args.engine, args.profile
    phase = os.environ.get("DURABLE_JOBS_PHASE", "diagnostic")
    if phase not in {"diagnostic", "warmup", "measured"} or (profile == "dense" and phase != "diagnostic"):
        raise ValueError("invalid phase for this profile")
    if args.local_smoke and phase != "diagnostic":
        raise ValueError("local component smoke requires diagnostic phase")
    before = host_snapshot()
    h.write_json(out / "host-before.json", before)
    if not args.local_smoke:
        if before["system"] != "Linux" or before["architecture"] != "x86_64":
            raise RuntimeError("benchmark requires native x86-64 Linux")
        if len(before["cpus"]) != 2 or before["host_logical_cpus"] != 4:
            raise RuntimeError("expected two pinned CPUs on the four-CPU Linode")

    manifest_path = artifacts / f"{engine}.json"
    build = json.loads(manifest_path.read_text())
    if not args.local_smoke and (build["system"], build["architecture"]) != ("Linux", "x86_64"):
        raise RuntimeError("expected a native x86-64 Linux build")
    command = artifacts / ("mesh" if engine == "mesh" else "oban")
    for name, checksum in build["files"].items():
        if h.digest(artifacts / name) != checksum:
            raise RuntimeError(f"artifact failed SHA-256 check: {name}")
    h.write_json(out / "build.json", build)
    h.write_json(out / "invocation.json", dict(engine=engine, profile=profile, phase=phase,
                                               local_smoke=args.local_smoke))
    if engine == "mesh":
        if not re.fullmatch(r"[0-9a-f]{40}", build["version"]) or build.get("base_revision") != MESH_REVISION:
            raise RuntimeError("wrong Mesh runtime lineage")
        if not args.local_smoke and build["version"] == MESH_REVISION:
            raise RuntimeError("dbcb6dbc compiled Jobs effects require Linux GNU ARM64")

    runners, slots, lanes, cap, _, checksum = PROFILES[profile]
    settings = json.loads(h.CONFIG.read_text())
    workload = settings["workloads"]["noop" if profile == "dense" else profile]
    batches = h.inputs(workload, slots, settings["seed"])
    scratch = Path(tempfile.mkdtemp(dir=state / "scratch", prefix="arm-"))
    cluster = None
    try:
        cluster = Cluster(scratch, out, cap)
        cluster.runners, cluster.slots, cluster.lanes, cluster.hold_ms = runners, slots, lanes, workload["hold_ms"]
        inetrc = artifacts / "oban.inetrc"
        cluster.proof_config = {
            "oban_erl_flags": settings["oban_erl_flags"],
            "oban_erl_inetrc": str(inetrc),
            "hashes": {"proof/oban.inetrc": INETRC_SHA256},
        }
        cluster.start()
        env = dict(cluster.env, MESH_SCHEDULER_MIN_WORKERS="2", MESH_SCHEDULER_MAX_WORKERS="2")
        if engine == "oban":
            if h.digest(inetrc) != INETRC_SHA256:
                raise RuntimeError("wrong Oban resolver file")
            env = dict(env, ERL_FLAGS=settings["oban_erl_flags"])
            versions = h.checked([str(command), "versions"], env=env, log=out / "oban-versions.log")
            if json.loads(versions.splitlines()[-1]) != VERSIONS:
                raise RuntimeError("wrong Oban runtime versions")
            if sys.platform == "linux":
                preflight = h.checked([str(command), "preflight"], env=env, log=out / "oban-preflight.log")
                report = regression.oban_runtime.report_from(preflight)
                regression.oban_runtime.verify_report(report, cluster.proof_config, "preflight")
                h.write_json(out / "oban-preflight.json", report)
        else:
            env.update(JOBS_PARITY_MESH_BATCH="1", JOBS_PARITY_MESH_LISTEN="1",
                       MESH_EFFECT_LANES=str(lanes), MESH_EFFECT_APP_URL=cluster.url("mesh", "compiled_caller"),
                       MESH_DIAGNOSTIC_JOB_DISABLE_THP="1")
        subject = Subject(engine, [str(command)], artifacts, cluster, env)
        metrics = h.run_trial(subject, h.allocate_slots(runners, slots), workload,
                              batches, scratch, out, settings, 0)
        if metrics["input_sha256"] != checksum or metrics["peak_connections"] > cap:
            raise RuntimeError("frozen input digest or connection budget failed")
        if metrics["completed"] != sum(len(batch["jobs"]) for batch in batches):
            raise RuntimeError("completed job count differs from frozen input")
        if profile == "sparse" and metrics["peak_connections"] != (5 if engine == "mesh" else 4):
            raise RuntimeError("sparse sampled connection count differs from frozen topology")
        database = database_settings(cluster, engine)
        h.write_json(out / "postgres-settings.json", database)
        row = dict(metrics, profile=profile, phase=phase, runners=runners, slots=slots,
                   local_smoke=args.local_smoke, version=build["version"],
                   artifact_sha256=h.digest(manifest_path), connection_cap=cap,
                   reconciled_jobs=len(cluster.jobs), committed_work_records=2 * len(cluster.jobs),
                   durable_enqueue_transactions=len(cluster.receipts))
        common = {name: row[name] for name in (
            "input_sha256", "engine", "profile", "phase", "version", "artifact_sha256", "runners",
            "slots", "peak_executing", "peak_connections", "local_smoke",
        )}
        return row, dict(common, database=database, jobs=cluster.jobs, transactions=cluster.receipts,
                         runner_cpu_seconds=metrics["cpu_seconds_per_job"] * metrics["completed"],
                         peak_runner_rss_bytes=metrics["peak_rss_bytes"])
    except BaseException as error:
        if cluster:
            failure(out, error, cluster.password)
        raise
    finally:
        try:
            if cluster:
                # Retain scratch when PostgreSQL could not stop.
                cluster.stop()
            shutil.rmtree(scratch)
        finally:
            if cluster:
                redact(out, cluster.password)
            h.write_json(out / "host-after.json", host_snapshot())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--local-smoke", action="store_true", help="run a diagnostic development arm")
    parser.add_argument("engine", choices=("mesh", "oban"))
    parser.add_argument("profile", choices=PROFILES)
    parser.add_argument("evidence", type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    if os.getuid() == 0:
        parser.error("run as a non-root user; use run on the Linode")
    out = args.evidence
    if not out.is_absolute() or out.is_symlink():
        parser.error("use an absolute, non-symlink evidence directory")
    out.mkdir(exist_ok=True)
    if any(out.iterdir()):
        parser.error("use a fresh evidence directory")
    state = Path(os.environ.get("NATIVE_JOBS_STATE", ROOT / "target/linode-jobs-native")).resolve()
    artifacts = Path(os.environ.get("NATIVE_JOBS_ARTIFACTS", state / "artifacts")).resolve()
    (state / "scratch").mkdir(exist_ok=True)
    # Run SIGTERM through the same teardown as other trial failures.
    def terminate(signum, frame):
        raise SystemExit(128 + signum)
    signal.signal(signal.SIGTERM, terminate)
    try:
        row, data = run_arm(args, out, state, artifacts)
    except BaseException as error:
        if not (out / "failure.json").exists():
            failure(out, error)
        raise SystemExit(f"{type(error).__name__}: see {out / 'failure.json'}") from None
    h.write_json(out / "ledger.json", data)
    h.write_json(out / "row.json", row)


if __name__ == "__main__":
    main()
