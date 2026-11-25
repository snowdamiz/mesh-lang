"""Replay the sealed offline check from a relocated `git archive` export.

Usage, from the source repository:
    python3 -I verify-export.py EXPORT_ROOT REPO ORIGINAL_ROOT

EXPORT_ROOT must be a fresh `git archive` of a commit containing this directory.
REPO must contain b3557273d (see candidate-f3b3612d9.bundle). ORIGINAL_ROOT is the
checkout path recorded in upstream sealed intakes. The script only remaps that
prefix in memory, so no sealed file changes.
"""

import hashlib
import json
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
import tarfile

ID = "2026-11-25-public-large-exact-prepare"
PACKET = "c098809a67de34b2818d4bf5b8b10143e05e791e13b4f0290a36240f222bac70"
COMPOSITION = "b3557273d72be3802bf189b284e7caaf5f3caa0f"


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main(export, repo, original):
    here = export / "tools/durable-jobs-bench/results" / ID
    manifest = json.loads((here / "provenance/manifest.json").read_text())
    assert sha(here / "request.json") == manifest["packet"]["sha256"] == PACKET
    for target, entry in manifest["ignored_target_pins"].items():
        path = export / target
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(here / entry["copy"], path)
        assert sha(path) == entry["sha256"], target
    source = export / "target/2026-11-24-trusted-jobs-product-composition/source"
    source.mkdir(parents=True)
    archive = subprocess.run(["git", "-C", str(repo), "archive", COMPOSITION],
                             check=True, stdout=subprocess.PIPE).stdout
    subprocess.run(["tar", "-x", "-C", str(source)], input=archive, check=True)
    for name, bundle in manifest["bundles"].items():
        assert sha(here / name) == bundle["sha256"], name
        heads = subprocess.run(["git", "-C", str(repo), "bundle", "list-heads", str(here / name)],
                               check=True, capture_output=True, text=True).stdout.split()
        assert heads[0] == bundle["head"], name
        subprocess.run(["git", "-C", str(repo), "bundle", "verify", "-q", str(here / name)],
                       check=True, capture_output=True)
    request = json.loads((here / "request.json").read_text())
    with tarfile.open(here / "source.tar.gz") as tar:
        for name in request["portable_files"]:
            if not name.startswith("source/") or "/._" in name:
                continue
            committed = subprocess.run(["git", "-C", str(repo), "show",
                                        f"{request['revision']}:{name[len('source/'):]}"],
                                       check=True, capture_output=True).stdout
            assert tar.extractfile(name).read() == committed, name
    patch = subprocess.run(["git", "-C", str(repo), "diff", COMPOSITION, request["revision"]],
                           check=True, capture_output=True).stdout
    assert hashlib.sha256(patch).hexdigest() == sha(here / "candidate.patch")

    old, new = str(original), str(export)
    real_loads = json.loads

    def relocated(text, *args, **kwargs):
        if isinstance(text, (bytes, bytearray)):
            text = text.decode()
        return real_loads(text.replace(old, new), *args, **kwargs)

    json.loads = relocated
    sys.argv = [str(here / "controller.py"), "check"]
    runpy.run_path(str(here / "controller.py"), run_name="__main__")


if __name__ == "__main__":
    export, repo, original = (Path(value).resolve() for value in sys.argv[1:])
    main(export, repo, original)
