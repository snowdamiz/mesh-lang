"""Bridge the immutable current Jobs proof ELF to the frozen dense workload."""
from collections import Counter
import inspect
import json
import os
from pathlib import Path
import sys
from textwrap import dedent

from identity import observe, pinned_file, process_stamp

REV = "1e70f18d624a841740d3bbb3d0fd09b269257f68"
sys.path.insert(0, str(Path(__file__).resolve().parent / "overlay/replay"))
from backend import Subject as ProtectedSubject, arm


def replace_once(source, old, new):
    if source.count(old) != 1:
        raise ValueError(f"frozen adapter anchor changed: {old}")
    return source.replace(old, new)


def install():
    reconcile = arm.Cluster.reconcile_trial
    source = dedent(inspect.getsource(reconcile))
    source = replace_once(source, "claim_admitted_v3", "claim_batch_set_v1")
    source = replace_once(source, "super().reconcile_trial", "super(_frozen_cluster, self).reconcile_trial")
    scope = dict(reconcile.__globals__, _frozen_cluster=arm.Cluster)
    exec(compile(source, reconcile.__code__.co_filename, "exec"), scope)
    arm.Cluster.reconcile_trial = scope["reconcile_trial"]

    run_arm = arm.run_arm
    source = replace_once(inspect.getsource(run_arm),
                          'build.get("base_revision") != MESH_REVISION',
                          f'build["version"] != "{REV}"')
    scope = run_arm.__globals__
    exec(compile(source, run_arm.__code__.co_filename, "exec"), scope)

    parent = arm.regression.Subject
    original = parent.warm_producer.__wrapped__
    anchor = '    ) as broker, output.open("rb") as reader:\n'
    source = replace_once(dedent(inspect.getsource(original)), anchor,
                          anchor + "        self.broker_process = (broker, process_stamp(broker))\n")
    scope = dict(original.__globals__, process_stamp=process_stamp)
    exec(compile(source, original.__code__.co_filename, "exec"), scope)
    parent.warm_producer = scope["warm_producer"]


class Cluster(arm.Cluster):
    def configure_tls(self):
        super().configure_tls()
        with (self.data / "postgresql.conf").open("a") as stream:
            stream.write("work_mem = '4MB'\n")


class Subject(ProtectedSubject):
    @property
    def executable_sha(self):
        return os.environ["PRODUCT_ELF_SHA256"]

    def call(self, action, env, log):
        result = super().call(action, env, log)
        if action == "migrate":
            self.cluster.sql("mesh", """
                GRANT EXECUTE ON FUNCTION mesh_jobs.enqueue_batch_layout_v1(),
                    mesh_jobs.enqueue_batch_v1(text,jsonb) TO parity_producer;
            """)
            limits = json.loads(self.cluster.sql("mesh", """
                SELECT json_object_agg(rolname, rolconnlimit)
                FROM pg_roles WHERE rolname IN ('compiled_caller', 'parity_subject', 'parity_producer')
            """))
            storage = json.loads(self.cluster.sql("mesh", """
                SELECT json_build_object('args_storage', attstorage, 'work_mem', current_setting('work_mem'),
                    'max_connections', current_setting('max_connections')::int)
                FROM pg_attribute WHERE attrelid='mesh_jobs.jobs'::regclass AND attname='args'
            """))
            arm.h.write_json(self.cluster.evidence / "dense-setup.json", dict(storage=storage, limits=limits))
        return result

    def run(self, env, index, log):
        env = dict(env)
        if os.environ["HOT_CHECKOUT"] == "1":
            env["MESH_EFFECT_APP_URL"] += "&mesh_pool_hot_checkout=1"
        env["MESH_JOBS_CLAIM_LANE_LAUNCH"] = os.environ["MESH_JOBS_CLAIM_LANE_LAUNCH"]
        if os.environ["DURABLE_JOBS_PHASE"] == "diagnostic":
            env["MESH_JOBS_GRANT_DIAGNOSTICS"] = str(self.cluster.evidence / f"grant-ages-{index}.json")
        executable = Path(self.command[0])
        stamp = pinned_file(executable, self.executable_sha)
        child = super().run(env, index, log)
        try:
            child.identity = stamp, process_stamp(child)
        except BaseException:
            arm.h.stop_runners([child], require_clean=False)
            raise
        return child

    def capture_runners(self, processes, logs):
        super().capture_runners(processes, logs)
        broker, start = self.broker_process
        executable = Path(self.command[0])
        cpus = sorted(os.sched_getaffinity(0))
        launches = [observe(broker, executable, "session", self.executable_sha,
                            pinned_file(executable, self.executable_sha), start, cpus)]
        launches.extend(observe(child, executable, "run", self.executable_sha, *child.identity, cpus)
                        for child in processes)
        arm.h.write_json(self.cluster.evidence / "process-identity.json", launches)
        sessions = json.loads((self.cluster.evidence / "postgres-tls-sessions.json").read_text())
        counts = Counter(row["role"] for row in sessions)
        if counts["compiled_caller"] != 32 or counts["parity_producer"] != 1 or not 4 <= counts["parity_subject"] <= 8:
            raise RuntimeError("expected 32 warmed application TLS sessions and the fixed queue and producer roster")


if __name__ == "__main__":
    if sys.argv[1:3] != ["mesh", "noop"] or sorted(os.sched_getaffinity(0)) != [0, 1]:
        raise ValueError("expected native 2-CPU dense Mesh arm")
    artifact = Path(os.environ["NATIVE_JOBS_ARTIFACTS"])
    manifest = artifact / "mesh.json"
    if arm.h.digest(manifest) != os.environ["PRODUCT_MANIFEST_SHA256"]:
        raise ValueError("Jobs proof manifest changed")
    if json.loads(manifest.read_text())["files"]["mesh"] != os.environ["PRODUCT_ELF_SHA256"]:
        raise ValueError("Jobs proof ELF changed")
    _, slots, _, cap, memory, digest = arm.PROFILES["noop"]
    arm.PROFILES["noop"] = (2, slots, 16, cap, memory, digest)
    os.environ.update(TRUSTED_ADMISSION_POLICY="set_based", MESH_JOBS_SET_BASED_ENQUEUE="1")
    install()
    arm.Cluster, arm.Subject = Cluster, Subject
    arm.main()
