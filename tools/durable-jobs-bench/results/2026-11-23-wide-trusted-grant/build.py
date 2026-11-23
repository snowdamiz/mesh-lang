"""Build a private V1 diagnostic runner able to validate up to 32 grants."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
SOURCE = HERE.parent / "2026-11-23-ordinary-trusted-v1/ordinary-trusted-v1.tar.gz"
PARENT = HERE.parent / "2026-11-23-ordinary-runner-topology/local-pins.json"
BASE = "pub(super) const MAX_CLAIM_BATCH: usize = 16;"
WIDE = BASE.replace("16", "32")
AREA = ROOT / "target/wide-trusted-grant"


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def build(dest):
    dest.mkdir(parents=True)
    pins = json.loads(PARENT.read_text())
    assert sha(SOURCE) == pins["source_archive_sha256"]
    tree = dest / "src"
    tree.mkdir()
    with tarfile.open(SOURCE) as archive:
        archive.extractall(tree, filter="data")
    code = tree / "compiler/mesh-rt/src/jobs/store.rs"
    s = code.read_text()
    assert s.count(BASE) == 1 and s.count("MAX_CLAIM_BATCH") == 4
    original = sha(code)
    code.write_text(s.replace(BASE, WIDE))
    temp = dest / "tmp"
    temp.mkdir()
    env = dict(os.environ, TMPDIR=str(temp), CARGO_NET_OFFLINE="true")
    target = dest / "cargo"
    toolchain_bin = Path("/opt/homebrew/opt/rustup/bin")
    env["PATH"] = str(toolchain_bin) + os.pathsep + env["PATH"]
    cargo = toolchain_bin / "cargo"
    subprocess.run([cargo, "+1.99.0", "build", "--offline", "--locked", "-j", "4", "-p", "mesh-rt",
                    "--target-dir", target], cwd=tree, env=env, check=True)
    runtime = target / "debug/libmesh_rt.a"
    compiler = ROOT / "target/debug/meshc"
    stock = ROOT / "target/ordinary-runner-topology/local-artifacts"
    stock_manifest = json.loads((stock/"mesh.json").read_text())
    assert sha(compiler) == stock_manifest["compiler_sha256"] and sha(stock/"mesh") == pins["executable_sha256"]
    artifacts = dest / "artifacts"
    artifacts.mkdir()
    # Reuse the exact native archive and application source bytes from the control.
    for name, digest in stock_manifest["files"].items():
        if name != "mesh":
            assert sha(stock / name) == digest
            shutil.copy2(stock / name, artifacts / name)
    env["MESH_RT_LIB_PATH"] = str(runtime)
    subprocess.run([compiler, "build", artifacts, "--opt-level", "2", "--output", artifacts/"mesh"],
                   cwd=tree, env=env, check=True)
    recipe = subprocess.check_output(["git", "log", "-1", "--format=%H", "--", str(HERE/"build.py")],
                                     cwd=ROOT, text=True).strip()
    assert len(recipe) == 40
    changes = dict(source_archive_sha256=sha(SOURCE), store_before_sha256=original,
                   store_after_sha256=sha(code), recipe_commit=recipe, validated_claim_cap=32,
                   installed_default_sql_limit=16)
    manifest = {**stock_manifest, "version": recipe, "runtime_sha256": sha(runtime),
                "files": {k: sha(artifacts/k) for k in stock_manifest["files"]}, "experiment": changes}
    (artifacts / "mesh.json").write_text(json.dumps(manifest, indent=2) + "\n")
    receipt = dict(source=changes, manifest_sha256=sha(artifacts/"mesh.json"), runtime_sha256=sha(runtime),
                   executable_sha256=sha(artifacts/"mesh"), compiler_sha256=sha(compiler))
    (dest/"built.json").write_text(json.dumps(receipt, indent=2) + "\n")


if __name__ == "__main__":
    dest = Path(sys.argv[1]).resolve()
    if not dest.is_relative_to(AREA) or dest == AREA or dest.exists():
        raise ValueError("use a fresh build directory under target/wide-trusted-grant")
    build(dest)
