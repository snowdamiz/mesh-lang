"""Seal build inputs and runnable artifacts before the host enters timing."""

import hashlib
import json
from pathlib import Path
import platform
import sys


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


selection, directory, revision, base_revision = sys.argv[1:]
out = Path(directory)
for engine in ("mesh", "oban") if selection == "both" else (selection,):
    files = [out / engine]
    if engine == "mesh":
        files += [out / "mesh-toolchain.txt", *sorted((out / "mesh-source").iterdir())]
    else:
        files += [out / "oban-entry.exs", out / "oban.inetrc", out / "oban-source/mix.lock",
                  *sorted((out / "oban-source/lib").glob("*.ex")),
                  *sorted(path for path in (out / "oban-source/_build/prod/lib").glob("*/ebin/*")
                          if path.is_file())]
    data = {"engine": engine, "system": platform.system(), "architecture": platform.machine(),
            "version": revision if engine == "mesh" else "2.24.2",
            "files": {str(file.relative_to(out)): digest(file) for file in files}}
    if engine == "mesh":
        data.update(base_revision=base_revision, fixture="linux-warm-replay-20261122")
    else:
        data.update(engine_module="Oban.Engines.Basic", otp="29.1.1", elixir="1.20.4",
                    notifier="Oban.Notifiers.Isolated", fixture_revision="8526a5bd70a1cb37f8fc27611021c90a44a89590")
    (out / f"{engine}.json").write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")
