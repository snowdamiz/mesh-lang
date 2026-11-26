"""Emit the private source patch against the sealed ordinary trusted V2 bundle."""
import difflib
from pathlib import Path
import sys
import tarfile

source, archive = map(Path, sys.argv[1:])
files = [
    "compiler/mesh-rt/src/jobs/completion_writer.rs",
    "compiler/mesh-rt/src/jobs/runner.rs",
    "compiler/mesh-rt/src/jobs/trusted.rs",
    "fixture/main.mpl",
    "replay/manifest.py",
]
with tarfile.open(archive) as frozen:
    for name in files:
        try:
            original = frozen.extractfile(name).read().decode().splitlines(keepends=True)
        except KeyError:
            original = []
        candidate = (source / name).read_text().splitlines(keepends=True)
        print(f"diff --git a/{name} b/{name}")
        if not original:
            print("new file mode 100644")
        sys.stdout.writelines(difflib.unified_diff(
            original, candidate, f"a/{name}" if original else "/dev/null", f"b/{name}"))
