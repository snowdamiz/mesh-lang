"""Adapt the frozen workload/ledger to the product's public SQL entry points."""

from collections import Counter
import inspect
import json
import os
from pathlib import Path
import sys
import subprocess
from textwrap import dedent

FROZEN = Path(os.environ["PRODUCT_FROZEN_HARNESS"]).resolve(strict=True)
sys.path.insert(0, str(FROZEN / "tools/durable-jobs-bench/native"))
import arm

from identity import observe, pinned_file, process_stamp

MODES = {
    "product-bulk": ("set_based", False),
    "product-group": ("success_cohort", False),
    "product-d2": ("success_cohort", True),
}
GRANTS = {
    "claim": "mesh_jobs.claim_batch_set_v1(text,text,jsonb,bigint,integer,bigint)",
    "group": "mesh_jobs.complete_batch_v1(text,uuid[],uuid[])",
}


PROTOCOL = "locked_prepare_pg17_v1"


def prepared():
    return os.environ.get("PRODUCT_CLAIM_MODE") == "prepared"


def install():
    # Keep the frozen statement/ledger checks while changing the SQL identifiers.
    original = arm.Cluster.reconcile_trial
    source = dedent(inspect.getsource(original))
    claim = "activate_batch_v1" if prepared() else "claim_batch_set_v1"
    for old, new in (("claim_admitted_v3", claim),
                     ("complete_admitted_v3", "complete_batch_v1")):
        assert source.count(old) == 1
        source = source.replace(old, new)
    owner = original.__closure__[original.__code__.co_freevars.index("__class__")].cell_contents
    assert owner is arm.Cluster and source.count("super().reconcile_trial") == 1
    source = source.replace("super().reconcile_trial", "super(_product_cluster_owner, self).reconcile_trial")
    scope = dict(original.__globals__, _product_cluster_owner=owner)
    exec(compile(source, original.__code__.co_filename, "exec"), scope)
    arm.Cluster.reconcile_trial = scope["reconcile_trial"]


# Use the direct launcher employed by the sealed r2 evidence.
def install_broker_observer():
    parent = arm.regression.Subject
    original = parent.warm_producer.__wrapped__
    source = dedent(inspect.getsource(original))
    anchor = '    ) as broker, output.open("rb") as reader:\n'
    assert source.count(anchor) == 1
    source = source.replace(anchor, anchor + "        self.broker_process = (broker, process_stamp(broker) if sys.platform == 'linux' else None)\n")
    source = source.replace(
        '    output = evidence / "broker-output.log"',
        '    env["JOBS_PARITY_FIXTURE_DIR"] = str(cluster.scratch.resolve())\n'
        '    output = evidence / "broker-output.log"',
    )
    launch = '[*self.command, "session"]'
    assert source.count(launch) == 1
    source = source.replace(launch, '[str(self.session_executable), "session"]')
    inline = '            response = request({"verb": "enqueue", "batch": batch})'
    read = '            batch = json.loads(path.read_text()) if self.engine == "mesh" else str(path)'
    assert source.count(read + "\n" + inline) == 1
    source = source.replace(read + "\n" + inline, """
            if os.environ["PRODUCT_SESSION_MODE"] == "file":
                fixture_dir = cluster.scratch.resolve()
                if not path.is_absolute() or path.parent.resolve() != fixture_dir or path.is_symlink():
                    raise ValueError("expected an absolute trusted batch path in private trial scratch")
                response = request({"verb": "enqueue_file", "batch": str(fixture_dir / path.name),
                                    "request_id": path.stem})
            else:
                batch = json.loads(path.read_text()) if self.engine == "mesh" else str(path)
                response = request({"verb": "enqueue", "batch": batch})
""".strip("\n"))
    scope = dict(original.__globals__, process_stamp=process_stamp)
    exec(compile(source, original.__code__.co_filename, "exec"), scope)
    parent.warm_producer = scope["warm_producer"]


class Cluster(arm.Cluster):
    def reconcile_trial(self, engine, namespace, enqueued_ids, handler_attempts, commits):
        result = super().reconcile_trial(engine, namespace, enqueued_ids, handler_attempts, commits)
        details = json.loads(self.sql(engine, """
            SELECT json_build_object('files', temp_files, 'bytes', temp_bytes)
            FROM pg_stat_database WHERE datname = current_database()
        """))
        arm.h.write_json(self.evidence / "database-temp.json", details)
        return result

    def configure_tls(self):
        super().configure_tls()
        storage = os.environ["PRODUCT_STORAGE"]
        mem = {"default": "4MB", "external-4m": "4MB", "external-16m": "16MB"}[storage]
        with (self.data / "postgresql.conf").open("a") as stream:
            stream.write(f"work_mem = '{mem}'\n")


class Subject(arm.Subject):
    def run(self, env, index, log):
        env = dict(env)
        mode = os.environ.get("PRODUCT_COMPOSITION", "product-group")
        policy, hot = MODES[mode]
        env["TRUSTED_ADMISSION_POLICY"] = policy
        if prepared():
            env["MESH_JOBS_BATCH_PROTOCOL"] = PROTOCOL
        if hot:
            env["MESH_EFFECT_APP_URL"] += "&mesh_pool_hot_checkout=1"
        if sys.platform != "linux" or os.environ.get("PRODUCT_CAPTURE_IDENTITY") != "1":
            return super().run(env, index, log)
        executable = Path(self.command[0])
        stamp = pinned_file(executable, self.executable_sha)
        child = super().run(env, index, log)
        try:
            child.product_stamp = stamp, process_stamp(child)
        except BaseException:
            arm.h.stop_runners([child], require_clean=False)
            raise
        return child

    @property
    def session_executable(self):
        return Path(os.environ["PRODUCT_SESSION_EXECUTABLE"])

    @property
    def session_sha(self):
        return os.environ["PRODUCT_SESSION_SHA256"]

    @property
    def executable_sha(self):
        return os.environ["PRODUCT_ELF_SHA256"]

    def capture_runners(self, processes, logs):
        super().capture_runners(processes, logs)
        if sys.platform == "linux":
            executable = Path(self.command[0])
            cpus = sorted(os.sched_getaffinity(0))
            broker, start = self.broker_process
            assert broker not in processes
            session_executable = self.session_executable
            launches = [observe(broker, session_executable, "session", self.session_sha,
                                pinned_file(session_executable, self.session_sha), start, cpus)]
            launches += [observe(child, executable, "run", self.executable_sha,
                                 *child.product_stamp, cpus) for child in processes]
            arm.h.write_json(self.cluster.evidence / "process-identity.json", launches)
        tls = json.loads((self.cluster.evidence / "postgres-tls-sessions.json").read_text())
        counts = Counter(row["role"] for row in tls)
        assert counts["compiled_caller"] == self.cluster.lanes * self.cluster.runners
        assert counts["parity_producer"] == 1
        assert 2 <= counts["parity_subject"] <= 4 * len(processes)

    def call(self, action, env, log):
        result = super().call(action, env, log)
        if action == "migrate":
            product = Path(os.environ["PRODUCT_SOURCE"]).resolve(strict=True)
            script = product / "scripts/fixtures/jobs-role-grants.sql"
            sql = script.read_text().split("-- Optional producer fast path.")[0]
            sql = sql.replace(':"producer"', 'parity_producer').replace(':"worker"', 'parity_subject')
            sql = sql.replace(':"operator"', 'parity_operator').replace(':"inspector"', 'parity_inspector')
            self.cluster.sql("mesh", """
                CREATE ROLE parity_migrator NOLOGIN;
                CREATE ROLE parity_operator NOLOGIN;
                CREATE ROLE parity_inspector NOLOGIN;
                REVOKE parity_subject FROM parity_producer;
                REASSIGN OWNED BY parity_subject TO parity_migrator;
                GRANT CONNECT ON DATABASE mesh TO parity_subject, parity_producer, compiled_caller;
                GRANT SELECT ON public.mesh_jobs_parity_attempts, public.mesh_jobs_parity_effects,
                    public.mesh_jobs_parity_product, public.mesh_jobs_parity_authority TO parity_producer;
                GRANT INSERT ON public.mesh_jobs_parity_product TO parity_producer;
            """ + sql + f"""
                GRANT EXECUTE ON FUNCTION {GRANTS['claim']}, {GRANTS['group']} TO parity_subject;
                GRANT EXECUTE ON FUNCTION mesh_jobs.enqueue_batch_layout_v1(),
                    mesh_jobs.enqueue_batch_v1(text,jsonb) TO parity_producer;
            """)
            privileges = self.cluster.sql("mesh", f"""
                SELECT has_function_privilege('parity_subject', '{GRANTS['group']}', 'EXECUTE')
                   AND NOT has_function_privilege('parity_producer', '{GRANTS['group']}', 'EXECUTE')
                   AND NOT has_function_privilege('compiled_caller', '{GRANTS['group']}', 'EXECUTE')
                   AND NOT has_function_privilege('parity_producer', '{GRANTS['claim']}', 'EXECUTE')
            """)
            assert privileges == "t"
            if prepared():
                installer = Path(os.environ["PRODUCT_PREPARE_INSTALLER"]).resolve(strict=True)
                module = Path(os.environ["PRODUCT_PREPARE_MODULE"]).resolve(strict=True)
                args, pg_env = self.cluster.psql("mesh")
                subprocess.run([*args, "-v", "worker=parity_subject",
                                "-v", f"module_path={module}", "-f", str(installer)],
                               env=pg_env, check=True, capture_output=True)
                rights = self.cluster.sql("mesh", """
                    SELECT bool_and(has_function_privilege('parity_subject', f, 'EXECUTE')
                        AND NOT has_function_privilege('parity_producer', f, 'EXECUTE')
                        AND NOT has_function_privilege('compiled_caller', f, 'EXECUTE')
                        AND NOT (SELECT prosecdef FROM pg_proc WHERE oid = f))
                    FROM unnest(ARRAY[
                        'mesh_jobs.lock_exact_v1(text,uuid,xid,tid)'::regprocedure,
                        'mesh_jobs.prepare_batch_v1(text,text,jsonb,integer)'::regprocedure,
                        'mesh_jobs.activate_batch_v1(text,text,jsonb,bigint,xid8,jsonb,boolean)'::regprocedure
                    ]) AS f
                """)
                assert rights == "t"
                arm.h.write_json(self.cluster.evidence / "prepare-install.json",
                                 dict(protocol=PROTOCOL, roles_scoped=True))
            storage = os.environ.get("PRODUCT_STORAGE", "default")
            if storage != "default":
                self.cluster.sql("mesh", "ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTERNAL;")
            settings = json.loads(self.cluster.sql("mesh", """
                SELECT json_build_object('args_storage', a.attstorage, 'args_compression', a.attcompression,
                    'work_mem', current_setting('work_mem'),
                    'default_toast_compression', current_setting('default_toast_compression'),
                    'shared_buffers', current_setting('shared_buffers'),
                    'wal_compression', current_setting('wal_compression'),
                    'max_connections', current_setting('max_connections')::int)
                FROM pg_attribute a WHERE a.attrelid='mesh_jobs.jobs'::regclass AND a.attname='args'
            """))
            assert settings["args_storage"] == ("x" if storage == "default" else "e")
            assert settings["work_mem"] == ("16MB" if storage == "external-16m" else "4MB")
            settings["storage"] = storage
            settings["topology"] = os.environ.get("PRODUCT_TOPOLOGY", "stock")
            arm.h.write_json(self.cluster.evidence / "storage-topology.json", settings)
        return result
