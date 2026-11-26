"""Apply the experiment switches to the frozen public dense adapter."""
import os
from pathlib import Path
import sys

import base_adapter as base
from logger import verify_dense_logs

MODES = {"hot-sync": (True, False), "hot-ready": (True, True), "cold-ready": (False, True)}
mode = os.environ["CHECKOUT_MODE"]
hot, readiness = MODES[mode]
trace = os.environ.get("DURABLE_JOBS_PHASE") == "diagnostic"
os.environ["HOT_CHECKOUT"] = str(int(hot))


class Subject(base.Subject):
    def run(self, env, index, log):
        env = dict(env)
        if readiness:
            env["MESH_EFFECT_APP_URL"] += "&mesh_cooperative_pool=1"
            if trace:
                env["MESH_PG_TRACE_WAITS"] = "1"
        return super().run(env, index, log)

    def capture_runners(self, processes, logs):
        super().capture_runners(processes, logs)
        base.arm.h.write_json(self.cluster.evidence / "readiness-mode.json",
                              dict(mode=mode, trace=trace, pool_cap=16))


if __name__ == "__main__":
    if sys.argv[1:3] != ["mesh", "noop"] or sorted(os.sched_getaffinity(0)) != [0, 1]:
        raise ValueError("expected native two-CPU dense arm")
    artifact = Path(os.environ["NATIVE_JOBS_ARTIFACTS"])
    manifest = artifact / "mesh.json"
    if base.arm.h.digest(manifest) != os.environ["PRODUCT_MANIFEST_SHA256"]:
        raise ValueError("candidate manifest changed")
    if base.json.loads(manifest.read_text())["files"]["mesh"] != os.environ["PRODUCT_ELF_SHA256"]:
        raise ValueError("candidate ELF digest differs from its manifest")
    _, slots, _, cap, memory, digest = base.arm.PROFILES["noop"]
    base.arm.PROFILES["noop"] = (2, slots, 16, cap, memory, digest)
    os.environ.update(TRUSTED_ADMISSION_POLICY="set_based", MESH_JOBS_SET_BASED_ENQUEUE="1")
    base.install()
    base.arm.Cluster, base.arm.Subject = base.Cluster, Subject
    base.arm.main()
    out = Path(sys.argv[3])
    verify_dense_logs(lambda name: base.json.loads((out / f"{name}.json").read_text()),
                      lambda name: (out / name).read_text())
