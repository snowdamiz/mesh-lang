"""Cross-build only the dense River benchmark adapter from pinned inputs."""

import gzip
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
SUITE = HERE.parents[1]
ROOT = SUITE.parents[1]
DENSE = HERE.parent / "2026-11-23-dense-matched-topology"
STOCK_COMMIT = "1617e257f26ec7eb3b5996406d0993918d482226"
EVIDENCE = HERE / "evidence"
BINARY = EVIDENCE / "river-linux-amd64.gz"
RECEIPT = EVIDENCE / "build.json"
FILES = ("db.go", "evidence.go", "main.go", "worker.go", "go.mod", "go.sum")


def read(path):
    return json.loads(path.read_text())


def sha(data):
    return hashlib.sha256(data).hexdigest()


def dump(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n")


def source(work):
    dense = read(DENSE / "river-build.json")
    for name in FILES:
        path = (SUITE / "go" / name).relative_to(ROOT)
        data = subprocess.check_output(["git", "show", f"{STOCK_COMMIT}:{path}"], cwd=ROOT)
        assert sha(data) == dense["stock_source"][str(path)]
        if name == "main.go":
            before, after = (dense["change"][key].encode() for key in ("before", "after"))
            assert data.count(before) == 1
            data = data.replace(before, after)
            assert sha(data) == dense["patched_main_sha256"]
        (work / name).write_bytes(data)
    subprocess.run(["patch", "-s", "-p1", "-i", HERE / "adapter.patch"], cwd=work, check=True)
    return {path.name: sha(path.read_bytes()) for path in sorted(work.iterdir())}


def build():
    assert not BINARY.exists() and not RECEIPT.exists()
    with tempfile.TemporaryDirectory(prefix="dense-river-readiness-", dir=ROOT / "target") as location:
        work = Path(location)
        src = work / "go"
        src.mkdir()
        pins = source(src)
        env = dict(os.environ, CGO_ENABLED="0", GOOS="linux", GOARCH="amd64", GOAMD64="v1",
                   GOFLAGS="", GOTOOLCHAIN="local", GOWORK="off", GOPROXY="off",
                   GOCACHE=str(work / "cache"), GOTMPDIR=str(work), TMPDIR=str(work))
        compiler = subprocess.check_output(["go", "version"], env=env, text=True).strip()
        assert compiler.startswith("go version go1.26.4 ")
        binary = work / "river-bench"
        subprocess.run(["go", "build", "-mod=readonly", "-trimpath", "-buildvcs=false",
                        "-o", binary, "."], cwd=src, env=env, check=True)
        build_info = subprocess.check_output(["go", "version", "-m", binary], env=env,
                                             text=True).splitlines()[1:]
        dense = read(DENSE / "river-build.json")
        assert build_info == dense["build_info"]
        data = binary.read_bytes()
        EVIDENCE.mkdir(exist_ok=True)
        with BINARY.open("xb") as stream, gzip.GzipFile(filename="", fileobj=stream, mode="wb", mtime=0) as archive:
            archive.write(data)
        dump(RECEIPT, dict(stock_elf=dense["candidate_sha256"], compiler=compiler,
                           candidate_elf=sha(data), compressed_sha256=sha(BINARY.read_bytes()),
                           source_pins=pins, build_info=build_info))


def check():
    receipt = read(RECEIPT)
    data = gzip.decompress(BINARY.read_bytes())
    assert data[:6] == b"\x7fELF\x02\x01" and data[18:20] == b"\x3e\x00"
    assert sha(data) == receipt["candidate_elf"] != receipt["stock_elf"]
    assert sha(BINARY.read_bytes()) == receipt["compressed_sha256"]
    assert receipt["build_info"] == read(DENSE / "river-build.json")["build_info"]
    with tempfile.TemporaryDirectory(prefix="dense-river-source-", dir=ROOT / "target") as location:
        assert source(Path(location)) == receipt["source_pins"]
    return receipt


if __name__ == "__main__":
    build()
