#!/usr/bin/env python3
"""Freeze a side-by-side peer inventory and a content-addressed native package."""

import argparse
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import sys
import tarfile

from overlay import STOCK_SOURCE
from storage import write

HERE = Path(__file__).resolve().parent
FILES = ("oban-run", "overlay.py", "replay.py", "river.sql", "storage.py")
BASE = Path("/var/lib/durable-jobs-bench/native")
INSTALL = """#!/bin/bash
set -euo pipefail
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9</var/lib/durable-jobs-bench/native/native.lock
flock -n 9
umask 022
destination=DESTINATION
work=$(mktemp -d /var/lib/durable-jobs-bench/native/payload-storage-peers.partial.XXXXXX)
trap 'rm -rf -- "$work"' EXIT
cat >"$work/package.tar"
printf '%s  %s\\n' PACKAGE_SHA "$work/package.tar" | sha256sum -c
mkdir "$work/overlay"
tar --no-same-owner -xf "$work/package.tar" -C "$work/overlay"
if [[ -e "$destination" ]]; then
    diff -rq -- "$work/overlay" "$destination"
else
    mv -T -- "$work/overlay" "$destination"
fi
printf '%s\\n' "$destination"
"""


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def treated_inventory(stock, remote):
    inventory = deepcopy(stock)
    verified = {str(remote / name): digest(HERE / name) for name in FILES}
    for engine, launcher_sha256 in STOCK_SOURCE.items():
        spec = inventory["engines"][engine]
        argv = spec["argv"]
        if engine == "oban":
            program, artifact_env, launcher, name = argv
            if (program, name) != ("/usr/bin/env", "oban"):
                raise ValueError("unexpected stock Oban launcher arguments")
            stock_launcher = Path(launcher).with_name("arm.py")
            argv = [program, artifact_env, str(remote / "oban-run"), str(stock_launcher)]
        else:
            launcher, *options = argv
            stock_launcher = Path(launcher).with_suffix(".py")
            argv = ["/usr/bin/python3", "-B", str(remote / "overlay.py"), engine, str(stock_launcher), *options]
        pins = spec["verified_files"]
        if pins.get(str(stock_launcher)) != launcher_sha256:
            raise ValueError(f"{engine}: inventory lacks the exact stock launcher pin")
        spec["argv"] = argv
        spec["verified_files"] = dict(pins, **verified)
        if "*.json" not in spec["evidence_globs"]:
            spec["evidence_globs"].append("payload-storage.json")
    return inventory


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stock", type=Path, help="original two-CPU four-engine inventory")
    parser.add_argument("output", type=Path, help="new local freeze directory")
    args = parser.parse_args()

    sys.path.insert(0, str(HERE.parents[1]))
    from run import check_inventory, require

    stock = json.loads(args.stock.read_text())
    check_inventory(stock)
    require(stock["cpus"] == [0, 1], "this comparison requires CPUs 0 and 1")
    out = args.output
    out.mkdir()
    package = out / "overlay.tar"
    with tarfile.open(package, "w", format=tarfile.USTAR_FORMAT) as archive:
        for name in FILES:
            info = tarfile.TarInfo(name)
            info.size = (HERE / name).stat().st_size
            info.mode = 0o755 if name == "oban-run" else 0o644
            with (HERE / name).open("rb") as stream:
                archive.addfile(info, stream)
    package_hash = digest(package)
    remote = BASE / f"payload-storage-peers-{package_hash[:16]}"
    treated = treated_inventory(stock, remote)
    check_inventory(treated)
    (out / "inventory.stock.json").write_bytes(args.stock.read_bytes())
    write(out / "inventory.external-peers.json", treated)
    (out / "install.remote.sh").write_text(INSTALL.replace("DESTINATION", str(remote))
                                                     .replace("PACKAGE_SHA", package_hash))
    write(out / "freeze.json", {
        "overlay_archive_sha256": package_hash,
        "stock_inventory_sha256": digest(out / "inventory.stock.json"),
        "external_inventory_sha256": digest(out / "inventory.external-peers.json"),
        "install_command_sha256": digest(out / "install.remote.sh"),
        "native_path": str(remote),
    })


if __name__ == "__main__":
    main()
