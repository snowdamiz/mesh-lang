"""Execute the exact staged launcher over the sealed inputs without a workload."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile

import driver

HERE = Path(__file__).resolve().parent
ROOT = driver.dense.ROOT
IMAGE = "sha256:3dc01f342e31ad8de39338d14ab12de0090c9102b34f75ca9774ff1f4f5e904e"
SCRIPT = """
set -euo pipefail
base=/var/lib/durable-jobs-bench/native
mkdir -p "$base"
cp -R /fixture/. "$base/"
stage=/var/tmp/mesh-durable-jobs-matrix/stage
mkdir -p "${stage%/*}"
mv "$base/stage" "$stage"
out="$stage/raw/check/adapter"
mkdir -p "$out"
touch "$base/native.lock"
chmod 0711 "$base" "${stage%/*}" "$stage" "$stage/raw" "$stage/raw/check"
chmod 0600 "$stage/topology.py"
export DURABLE_JOBS_PHASE=measured
export PRODUCT_ELF_SHA256="$1" PRODUCT_MANIFEST_SHA256="$2"
for pair in '2 2' '1 4' '1 2'; do
    read -r runners workers <<< "$pair"
    /bin/bash "$stage/native-topology.sh" "$runners" "$workers" mesh noop "$out" --stage-check
    test -z "$(find "$out" -mindepth 1 -maxdepth 1 -print -quit)"
done
"""


def store(root, name, data):
    path = root / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def main():
    base, retry = driver.base, driver.retry
    binding = retry.relink_binding()
    with tempfile.TemporaryDirectory(prefix="dense-dispatch-stage-", dir=ROOT / "target") as tmp:
        fixture = Path(tmp)
        with tarfile.open(base.SOURCE / "source.tar.gz") as archive:
            get = driver.dense.archive_files(archive)
            for name in ("overlay/replay/backend.py", "source/scripts/fixtures/jobs-role-grants.sql"):
                store(fixture / base.PROOF_STAGE.name, name, get(name))
            pins = get("overlay/frozen-harness.sha256").decode().splitlines()
        with tarfile.open(base.HARNESS_ARCHIVE) as archive:
            get = driver.dense.archive_files(archive, "ordinary-v2/")
            for pin, name in map(str.split, pins):
                data = get(name)
                assert hashlib.sha256(data).hexdigest() == pin
                store(fixture / driver.dense.FROZEN.parent.name, "ordinary-v2/" + name, data)
        with tarfile.open(retry.OUT / "setup/raw.tar.gz") as archive:
            manifest = driver.dense.archive_files(archive)("mesh.json")
            store(fixture / retry.STAGE.name, "artifacts/jobs/mesh.json", manifest)
        for name in ("adapter.py", "identity.py"):
            store(fixture / retry.STAGE.name, name, retry.ADAPTER_FILES[name].read_bytes())
        (fixture / "stage").mkdir()
        names = ("native-topology.sh", "topology.py")
        for name in names:
            shutil.copyfile(HERE / name, fixture / "stage" / name)
        cmd = ["docker", "run", "--rm", "--pull=never", "--network", "none", "--cpuset-cpus", "0,1",
               "-v", f"{fixture}:/fixture:ro", "--entrypoint", "/bin/bash", IMAGE, "-c", SCRIPT, "--",
               binding["manifest"]["files"]["mesh"], binding["manifest_sha256"]]
        result = subprocess.run(cmd, check=True, text=True, stdout=subprocess.PIPE, timeout=45)
        rows = [json.loads(line) for line in result.stdout.splitlines()]
        assert [(row["profile"][0], row["runner_workers"]) for row in rows] == [(2, 2), (1, 4), (1, 2)]
        assert all(row["uid"] == 65534 and row["cpus"] == [0, 1] and row["broker_workers"] == 2
                   and row["profile"][1:4] == [64, 32 // row["profile"][0], 96] for row in rows)
        print(json.dumps(dict(image=IMAGE, environment="Linux ARM64", workload_ran=False,
                              lanes=rows, sha256={name: driver.run.digest(HERE / name) for name in names}),
                         indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
