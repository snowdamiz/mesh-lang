"""Run one retained dense V1 correctness arm with stack calls counted in runners only."""
import argparse
import importlib.util
import os
from pathlib import Path
import sys

p = argparse.ArgumentParser(description=__doc__)
for arg in ("source", "artifacts", "state", "interposer", "out"):
    p.add_argument(arg, type=lambda value: Path(value).resolve())
a = p.parse_args()
driver = a.source / "tools/durable-jobs-bench/native/arm.py"
sys.path.insert(0, str(driver.parent))
spec = importlib.util.spec_from_file_location("archived_v1", driver)
arm = importlib.util.module_from_spec(spec)
spec.loader.exec_module(arm)

class Counted(arm.Subject):
    def run(self, env, index, log):
        return super().run(dict(env, DYLD_INSERT_LIBRARIES=str(a.interposer)), index, log)

arm.Subject = Counted
os.environ.update(NATIVE_JOBS_STATE=str(a.state), NATIVE_JOBS_ARTIFACTS=str(a.artifacts),
                  DURABLE_JOBS_PHASE="diagnostic")
os.environ.pop("JOBS_PARITY_MESH_EVENTS", None)
sys.argv = [str(driver), "--local-smoke", "mesh", "dense", str(a.out)]
arm.main()
