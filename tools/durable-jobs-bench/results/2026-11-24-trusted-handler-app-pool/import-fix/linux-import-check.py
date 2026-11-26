"""Load the frozen app-pool adapter chain without entering run_arm or starting PostgreSQL."""
import os
from pathlib import Path
import runpy
import sys

stage, stock = map(Path, sys.argv[1:])
assert sys.platform == "linux" and os.getuid() == 65534 and sys.dont_write_bytecode
assert stage.stat().st_mode & 0o777 == 0o711 and stage.stat().st_uid == 0
sys.path[0] = str(stage)
try:
    list(stage.iterdir())
except PermissionError:
    pass
else:
    raise AssertionError("nobody unexpectedly listed stage")
entry = runpy.run_path(str(stage / "native-arm.py"))
assert Path(entry["topology"].__file__) == stage / "topology_arm.py"
arm = entry["load_arm"](stock)
native = stock / "tools/durable-jobs-bench/native"
proof = stock / "proof/jobs-parity/linux-regression"
assert Path(arm.__file__) == native / "arm.py"
assert Path(arm.ledger.__file__) == native / "ledger.py"
assert Path(arm.regression.oban_runtime.__file__) == proof / "oban_runtime.py"
assert Path(arm.regression.proof_transport.__file__) == proof / "proof_transport.py"
assert Path(arm.h.__file__) == stock / "scripts/jobs-parity.py"
assert arm.h.CONFIG == proof / "jobs-parity-profiles.json" or arm.h.CONFIG == stock / "scripts/jobs-parity-profiles.json"
assert arm.PROFILES["noop"] == entry["DENSE"] and arm.PROFILES["short"] == entry["SHORT"]
print("APP_POOL_LINUX_FULL_IMPORT_OK")
