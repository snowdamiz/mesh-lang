#!/usr/bin/env python3
"""Run the producer attribution with the retained native toolchain during SETUP."""

import argparse
import fcntl
import io
from pathlib import Path
import tarfile

from controller import ADAPTER, BASE, DIRECT, HERE, STOCK, run

FILES = ("prepare.py", "prove.py", "attribute.py")


def measure(output):
    output.mkdir()
    bundle = io.BytesIO()
    with tarfile.open(fileobj=bundle, mode="w") as archive:
        for name in FILES:
            archive.add(HERE / name, arcname=name)
    pins = "\n".join(f"{run.digest(HERE / name)}  {name}" for name in FILES)
    command = f"""
set -euo pipefail
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<{BASE}/native.lock
flock -n 9

scratch=$(mktemp -d {BASE}/scratch/direct-v3-attribution.XXXXXX)
trap 'rm -rf -- "$scratch"' EXIT
tar -xf - -C "$scratch"
(cd "$scratch" && sha256sum -c <<'DIGESTS'
{pins}
DIGESTS
) >&2
mkdir "$scratch/tmp"
chown -R nobody:nogroup "$scratch"
taskset -c 0,1 /usr/sbin/runuser -u nobody -- env -i \
    PATH={BASE}/toolchain/llvm-21.1.8/bin:/usr/bin:/bin LC_ALL=C.UTF-8 \
    PG_BIN_DIR={BASE}/toolchain/pgsql-17.11/bin TMPDIR="$scratch/tmp" \
    python3 -B "$scratch/attribute.py" {DIRECT} {STOCK}/abi/meshc {STOCK}/abi/libmesh_rt.a \
    {ADAPTER} "$scratch" "$scratch/output" >&2
tar -czf - -C "$scratch/output" attribution.json link.log mesh.json broker-source/main.mpl \
    proof/proof.json proof/accepted-and-rollback/broker-errors.log proof/after-first-chunk/broker-errors.log
"""
    with (run.SUITE / ".timing.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        with (output / "raw.tar.gz").open("xb") as archive, (output / "stage.log").open("xb") as log:
            result = run.ssh(["bash", "-c", command], input=bundle.getvalue(),
                             stdout=archive, stderr=log, timeout=110)
            result.check_returncode()
    with tarfile.open(output / "raw.tar.gz", "r:gz") as archive:
        for name in ("attribution.json", "proof/proof.json"):
            with archive.extractfile(name) as stream:
                (output / Path(name).name).write_bytes(stream.read())
    (output / "archive.sha256").write_text(f'{run.digest(output / "raw.tar.gz")}  raw.tar.gz\n')


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path)
    measure(parser.parse_args().evidence)
