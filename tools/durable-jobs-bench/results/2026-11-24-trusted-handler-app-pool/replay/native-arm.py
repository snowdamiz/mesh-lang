"""Use the frozen V1 workload, database, broker, and observer with one runner swap."""
from collections import Counter
from copy import copy
import json
import os
from pathlib import Path
import sys
import time

# The controller stages the archived topology adapter beside this file.
from topology_arm import CAPS, SESSIONS, load_arm

DENSE = (8, 64, 4, 96, 2, "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a")
SHORT = (1, 16, 8, 20, 2, "ac0bcbf2465ea6d6807e8bfcdcfc3b1dacfa18b775e3f4c074257d66bfbba9d1")
MODES = ("v1", "service", "direct", "service-observed", "direct-observed")


def main(source, runner, mode, profile, out):
    if mode not in MODES or os.sched_getaffinity(0) != {0, 1}:
        raise ValueError("unexpected application-pool lane or affinity")
    arm = load_arm(source)
    if arm.PROFILES["noop"] != DENSE or arm.PROFILES["short"] != SHORT:
        raise ValueError("frozen V1 geometry changed")
    arm.PROFILES["noop"] = (2, 64, 16, *DENSE[3:])
    runners, slots, lanes, cap, _, _ = arm.PROFILES[profile]
    caps = {"parity_subject": 4 * runners, "compiled_caller": runners * lanes, "parity_producer": 1}
    expected = {file: arm.h.digest(runner / file) for file in ("mesh", "mesh.json")}
    build = json.loads((runner / "mesh.json").read_text())
    if (build["system"], build["architecture"], build["files"]["mesh"]) != ("Linux", "x86_64", expected["mesh"]):
        raise ValueError("unsealed runner artifact")
    for file, digest in build["files"].items():
        if arm.h.digest(runner / file) != digest:
            raise ValueError(f"runner input digest changed: {file}")
    if mode != "v1" and build["mode"] != mode:
        raise ValueError("runner manifest mode differs")

    class Cluster(arm.Cluster):
        counter_samples = 0

        def sessions(self, engine):
            return json.loads(self.sql(engine, SESSIONS))

        def audit_tls(self, engine):
            def warm():
                rows = self.sessions(engine)
                return Counter(row["role"] for row in rows if row["tls"]) == caps and len(rows) == sum(caps.values())

            arm.h.wait_for(warm, time.monotonic() + 10, "exact V1 warmed TLS allocation", 0.05)
            super().audit_tls(engine)
            arm.h.write_json(out / "sql-capacity.json", json.loads(self.sql(engine, CAPS)))

        def reconcile_trial(self, engine, namespace, enqueued_ids, handler_attempts, commits):
            result = super().reconcile_trial(engine, namespace, enqueued_ids, handler_attempts, commits)
            statements = json.loads(self.sql(engine, """
              SELECT coalesce(json_object_agg(query, calls), '{}') FROM pg_stat_statements
              WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database())
                AND userid = 'parity_producer'::regrole
                AND (query LIKE 'SELECT mesh_jobs.enqueue(%'
                  OR query LIKE '%mesh_jobs.enqueue_batch_v1(%')
            """))
            arm.h.write_json(out / "scalar-enqueues.json", statements)
            return result

        def calls(self, engine):
            self.counter_samples += 1
            if self.counter_samples == 2:
                arm.h.write_json(out / "sessions-before.json", self.sessions(engine))
            counters = super().calls(engine)
            arm.h.write_json(out / f"counters-{self.counter_samples}.json", counters)
            if self.counter_samples == 3:
                arm.h.write_json(out / "sessions-end.json", self.sessions(engine))
            return counters

    original_subject = arm.Subject

    class Subject(original_subject):
        def run(self, env, index, log):
            selected = copy(self)
            selected.command = [str(runner / "mesh")]
            selected.cwd = runner
            return original_subject.run(selected, env, index, log)

        def capture_runners(self, processes, logs):
            super().capture_runners(processes, logs)
            identities = []
            for child in processes:
                mounted = Path(f"/proc/{child.pid}/exe")
                if not mounted.samefile(runner / "mesh"):
                    raise RuntimeError("live runner executable differs from its seal")
                identities.append({"pid": child.pid, "executable": str(mounted.resolve())})
            arm.h.write_json(out / "runner-identity.json", dict(
                mode=mode, runners=identities, hashes=expected, build=build,
                queues_per_runner=1, slots_per_runner=slots // runners, app_sessions_per_runner=lanes,
                postgres_cap=cap, scheduler_min=int(self.env["MESH_SCHEDULER_MIN_WORKERS"]),
                scheduler_max=int(self.env["MESH_SCHEDULER_MAX_WORKERS"]),
                batch=self.env["JOBS_PARITY_MESH_BATCH"], listen=self.env["JOBS_PARITY_MESH_LISTEN"]))

    original_run_arm = arm.run_arm

    def run_arm(*args):
        row, ledger = original_run_arm(*args)
        for receipt in (row, ledger):
            receipt.update(version=build["version"], artifact_sha256=expected["mesh.json"])
        return row, ledger

    arm.Cluster, arm.Subject, arm.run_arm = Cluster, Subject, run_arm
    sys.argv = [str(source), "mesh", profile, str(out)]
    arm.main()


if __name__ == "__main__":
    source, runner, mode, profile, out = sys.argv[1:]
    main(Path(source), Path(runner), mode, profile, Path(out))
