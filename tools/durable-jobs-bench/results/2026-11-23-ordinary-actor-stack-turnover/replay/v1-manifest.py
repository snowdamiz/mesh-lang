import hashlib
import json
from pathlib import Path
import platform
import sys

out, compiler, runtime = map(Path, sys.argv[1:])
def sha(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()

(out / "mesh.json").write_text(json.dumps(dict(
    system=platform.system(), architecture=platform.machine(),
    version="3828f20d2d6beb683868cce33e6dd1958365914b",
    base_revision="dbcb6dbc1de3a6887fc3757c2b91f7974d74ed17",
    execution_contract="ordinary-trusted-shared-process",
    compiler_sha256=sha(compiler), runtime_sha256=sha(runtime),
    files={name: sha(out / name) for name in (
        "mesh", "main.mpl", "writes.mpl", "worker_identity.mpl",
        "worker_identity.c", "libworker_identity.a", "mesh.toml")},
), indent=2) + "\n")
