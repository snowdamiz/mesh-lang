#!/usr/bin/env python3
"""Reproduce the frozen River ELF, then build the single opt-in Linux ELF."""

import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
SUITE = HERE.parents[1]
ROOT = SUITE.parents[1]
SOURCE = SUITE / "go"
STOCK_COMMIT = "1617e257f26ec7eb3b5996406d0993918d482226"
STOCK_ELF = "5db25a75ccfbf992cb72479a86470baa3a1ac054b5d780e0854e41caaaadf4d0"
HISTORICAL = HERE.parent / "2026-11-24-exit-confirmed-bulk-d2"


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def build(destination):
    tool = shutil.which("go")
    if not tool:
        raise RuntimeError("Go 1.26.4 is required")
    cold = json.loads((HISTORICAL / "controller-packet.json").read_text())["native_cold_pins"]["files"]
    pins = {name.split("/source/", 1)[1]: digest for name, digest in cold.items()
            if "/river/builds/" in name and "/source/" in name and not name.endswith("/SHA256SUMS")}
    candidate_files = {path.name: sha(path) for path in sorted(SOURCE.iterdir())
                       if path.suffix == ".go" or path.name in {"go.mod", "go.sum"}}
    for name, expected in pins.items():
        if Path(name).name not in {"main.go", "worker.go"} and sha(ROOT / name) != expected:
            raise RuntimeError(f"frozen source changed: {name}")
    (ROOT / "target").mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="river-readiness-build-", dir=ROOT / "target") as location:
        work = Path(location)
        source = work / "go"
        source.mkdir()
        for name, expected in pins.items():
            if "/go/" in name and Path(name).name in candidate_files:
                saved = subprocess.check_output(["git", "show", f"{STOCK_COMMIT}:{name}"], cwd=ROOT)
                if hashlib.sha256(saved).hexdigest() != expected:
                    raise RuntimeError(f"frozen adapter source differs: {name}")
                (source / Path(name).name).write_bytes(saved)
        env = dict(os.environ, CGO_ENABLED="0", GOOS="linux", GOARCH="amd64", GOAMD64="v1",
                   GOFLAGS="", GOTOOLCHAIN="local", GOWORK="off", GOPROXY="off",
                   GOCACHE=str(work / "cache"), GOTMPDIR=str(work))
        compiler = subprocess.check_output([tool, "version"], env=env, text=True).strip()
        if not compiler.startswith("go version go1.26.4 "):
            raise RuntimeError(f"frozen Go compiler mismatch: {compiler}")
        command = [tool, "build", "-mod=readonly", "-trimpath", "-buildvcs=false", "-o"]
        stock, candidate = work / "stock", work / "river-bench"
        subprocess.run([*command, str(stock), "."], cwd=source, env=env, check=True)
        if sha(stock) != STOCK_ELF:
            raise RuntimeError("Go build did not reproduce the historical stock ELF")
        for name in candidate_files:
            shutil.copyfile(SOURCE / name, source / name)
        subprocess.run([*command, str(candidate), "."], cwd=source, env=env, check=True)
        build_info = subprocess.check_output([tool, "version", "-m", str(candidate)], env=env,
                                             text=True).splitlines()[1:]
        destination.mkdir(parents=True, exist_ok=True)
        packaged = destination / "river-linux-amd64.gz"
        with packaged.open("xb") as stream, gzip.GzipFile(filename="", fileobj=stream, mode="wb", mtime=0) as archive:
            archive.write(candidate.read_bytes())
        result = dict(stock_commit=STOCK_COMMIT, stock_rebuild_sha256=sha(stock),
                      candidate_sha256=sha(candidate), compressed_sha256=sha(packaged),
                      source_pins=candidate_files, stock_source_pins=pins, compiler=compiler,
                      build_info=build_info)
        with (destination / "build.json").open("x") as stream:
            json.dump(result, stream, indent=2, sort_keys=True)
            stream.write("\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    build(parser.parse_args().destination.resolve())
