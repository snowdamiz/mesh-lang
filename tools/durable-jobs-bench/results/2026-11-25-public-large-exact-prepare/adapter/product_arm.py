"""Native-only product arm with the prior broker protocol and workload."""

import json
import os
from pathlib import Path
import sys

from backend import Cluster, MODES, Subject, arm, install, install_broker_observer

if __name__ == "__main__":
    assert sys.platform == "linux" and sys.argv[1] == "mesh"
    profile = sys.argv[2]
    assert profile in ("short", "noop", "sparse", "slow", "large")
    topology = os.environ["PRODUCT_TOPOLOGY"]
    assert topology == "stock" or (profile == "noop" and topology == "two-runner")
    if topology == "two-runner":
        _, slots, app_sessions, cap, memory, digest = arm.PROFILES[profile]
        arm.PROFILES[profile] = (2, slots, 4 * app_sessions, cap, memory, digest)
    assert os.environ["PRODUCT_STORAGE"] == "default" or profile in ("slow", "large")
    assert sorted(os.sched_getaffinity(0)) == [0, 1]
    mode = os.environ["PRODUCT_COMPOSITION"]
    os.environ.update(TRUSTED_ADMISSION_POLICY=MODES[mode][0],
                      MESH_JOBS_SET_BASED_ENQUEUE="1", PRODUCT_CAPTURE_IDENTITY="1")
    artifact = Path(os.environ["NATIVE_JOBS_ARTIFACTS"])
    manifest = artifact / "mesh.json"
    assert arm.h.digest(manifest) == os.environ["PRODUCT_MANIFEST_SHA256"]
    build = json.loads(manifest.read_text())
    assert build["files"]["mesh"] == os.environ["PRODUCT_ELF_SHA256"]
    install()
    install_broker_observer()
    arm.Cluster = Cluster
    arm.Subject = Subject
    arm.main()
