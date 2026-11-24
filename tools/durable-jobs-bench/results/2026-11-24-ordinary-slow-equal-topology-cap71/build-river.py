"""Cross-build the pinned River executable with a single slow profile change."""

import argparse
import difflib
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

PACKET = Path(__file__).resolve().parent
INPUT = "42bb9e2be94c17b36110081430faf2647a04f3a3e0f2decf00e7c216fed462fd"
BEFORE = f'"slow":   {{1, 64, 68, true, "{INPUT}"}},'
AFTER = BEFORE.replace("{1, 64, 68,", "{2, 64, 71,")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def build(source, destination, cache):
    old = json.loads((PACKET.parent / "2026-11-23-dense-matched-topology/river-build.json").read_text())
    pins = old["stock_source"]
    for name, checksum in pins.items():
        if digest(source / name) != checksum:
            raise ValueError(f"River source differs {name}")
    copied = destination / "river-src"
    shutil.copytree(source / "tools/durable-jobs-bench/go", copied)
    env = dict(os.environ, CGO_ENABLED="0", GOOS="linux", GOARCH="amd64", GOAMD64="v1",
               GOPROXY="off", GOMODCACHE=str(cache), GOCACHE=str(destination / "go-cache"),
               GOTOOLCHAIN="local", GOWORK="off", GOFLAGS="", GOENV="off", GOTMPDIR=str(destination / "tmp"),
               TMPDIR=str(destination / "tmp"))
    version = subprocess.check_output(["go", "version"], env=env, text=True).strip()
    if not version.startswith("go version go1.26.4 "):
        raise ValueError(f"unexpected toolchain {version}")
    command = ["go", "build", "-mod=readonly", "-trimpath", "-buildvcs=false", "-o"]
    stock, candidate = (destination / name for name in ("river-stock-rebuilt", "river-slow-2x32-cap71"))
    subprocess.run([*command, str(stock), "."], env=env, cwd=copied, check=True)
    if digest(stock) != old["stock_rebuild_sha256"]:
        raise ValueError("River stock rebuild did not match the installed ELF")
    main = copied / "main.go"
    original = main.read_text()
    if original.count(BEFORE) != 1:
        raise ValueError("the exact slow profile line is absent")
    revised = original.replace(BEFORE, AFTER, 1)
    main.write_text(revised)
    subprocess.run([*command, str(candidate), "."], env=env, cwd=copied, check=True)
    diff = "".join(difflib.unified_diff(original.splitlines(keepends=True), revised.splitlines(keepends=True),
                                       fromfile="a/main.go", tofile="b/main.go"))
    (destination / "river-profile.diff").write_text(diff)
    info = subprocess.check_output(["go", "version", "-m", str(candidate)], env=env, text=True).splitlines()
    return dict(stock_sha256=digest(stock), candidate_sha256=digest(candidate), go_version=version,
                stock_source=pins, before=BEFORE, after=AFTER, patched_main_sha256=digest(main),
                patch_sha256=digest(destination / "river-profile.diff"),
                call=command, build_info=info[1:])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("module_cache", type=Path)
    args = parser.parse_args()
    print(json.dumps(build(args.source.resolve(), args.destination.resolve(), args.module_cache.resolve()),
                     indent=2, sort_keys=True))
