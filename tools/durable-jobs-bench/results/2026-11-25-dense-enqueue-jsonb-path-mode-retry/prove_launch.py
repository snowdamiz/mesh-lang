"""Exercise the sealed launch and real runuser boundary in an offline Linux container."""

import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
from tempfile import TemporaryDirectory

HERE = Path(__file__).resolve().parent
ORIGINAL = HERE.with_name(HERE.name.removesuffix("-mode-retry"))
ROOT = HERE.parents[3]
IMAGE = "sha256:3dc01f342e31ad8de39338d14ab12de0090c9102b34f75ca9774ff1f4f5e904e"
PROFILE = "tools/durable-jobs-bench/results/2026-11-24-trusted-jobs-product-composition/replay"
PUBLIC_FILES = ("native-run", "backend.py", "identity.py", "product_arm.py")
BASE = "/var/lib/durable-jobs-bench/native"
SOURCE = f"{BASE}/source-trusted-jobs-product-deb2a3bc49fe/source"

# The native launcher drops the environment and switches users before Python.
# Stop just before the first product code executes. No database or ELF is present.
PROBE = """
import json
import os
from pathlib import Path
import pwd
import stat
import sys


class Cluster:
    pass


class Subject:
    pass


def stop_before_product(event, args):
    if event != "exec" or Path(args[0].co_filename).name != "product_arm.py":
        return
    stage = Path(args[0].co_filename).parent
    assert stage.parent == Path("/var/tmp")
    assert os.geteuid() == pwd.getpwnam("nobody").pw_uid
    assert Path.cwd() == stage and sys.argv[1:3] == ["mesh", "noop"]
    assert os.environ["DURABLE_JOBS_PHASE"] == "warmup"
    assert os.environ["JOBS_GROUPED_WRITER"] == "fixed8ms"
    assert "LOCAL_LAUNCH_PROOF_PRIVATE" not in os.environ
    output = Path(sys.argv[3])
    assert output.stat().st_uid == os.geteuid()
    assert stat.S_IMODE(output.stat().st_mode) == 0o700
    entry = stage / "factorial_arm.py"
    mode = stat.S_IMODE(entry.stat().st_mode)
    assert entry.stat().st_uid == 0 and mode == 0o644
    assert all(os.access(stage / name, os.R_OK) for name in (
        "backend.py", "identity.py", "contract.py", "overlay.py", "enqueue-stock.sql", "candidate.sql"))
    print(json.dumps({"effective_uid": os.geteuid(), "python_mode": oct(mode),
                      "disposition": "stopped before product code",
                      "projected": "overlay.apply_writer(cluster, True)" in entry.read_text()},
                     sort_keys=True), flush=True)
    raise SystemExit(0)


sys.addaudithook(stop_before_product)
"""

BOOTSTRAP = r"""
set -euo pipefail
base="$1"
source="$2"
profile="$3"
umask 022
launcher="$5"
treatment="$6"
mkdir -m 0755 -p "$source/$profile"
for name in native-run backend.py identity.py product_arm.py; do
    install -m 0644 "/packet/$name" "$source/$profile/$name"
done
touch "$base/native.lock"
frozen="$base/source-bulk-enqueue-3b09dc3d0e23/ordinary-v2/tools/durable-jobs-bench/native"
mkdir -m 0755 -p "$frozen"
install -m 0644 /packet/entry_probe.py "$frozen/arm.py"
if [[ ! -x /usr/bin/python3 ]]; then
    ln -s "$(command -v python3)" /usr/bin/python3
fi
mkdir /var/tmp/dense-enqueue-mode-proof-output
export PRODUCT_ARTIFACT="$base/builds/trusted-jobs-product-deb2a3bc49fe"
export PRODUCT_COMPOSITION=product-d2 PRODUCT_TOPOLOGY=two-runner PRODUCT_STORAGE=default
export PRODUCT_ELF_SHA256=d56d0e6112a94ff601bebf6f777061068ed2fe0f2f2da586963dd6e1326a220a
export PRODUCT_MANIFEST_SHA256=981846a3311edd9598f9a10e42e48d3ccf20329c85cfa60dac33d2544b237f7d
export JOBS_GROUPED_WRITER=fixed8ms DURABLE_JOBS_PHASE=warmup LOCAL_LAUNCH_PROOF_PRIVATE=yes
umask "$4"
set +e
bash "/packet/$launcher" "$source/$profile/native-run" "$treatment" mesh noop /var/tmp/dense-enqueue-mode-proof-output
status=$?
set -e
shopt -s nullglob
stages=(/var/tmp/dense-enqueue-jsonb-path.*)
[[ "${#stages[@]}" == 0 ]]
exit "$status"
"""


def verify():
    home = Path.home()
    docker = shutil.which("docker") or str(home / ".docker/bin/docker")
    socket = home / ".docker/run/docker.sock"
    if not socket.is_socket():
        socket = Path("/var/run/docker.sock")
    options = {"env": {key: value for key, value in os.environ.items()
                       if key not in {"DOCKER_CONTEXT", "DOCKER_HOST"}}}
    assert socket.is_socket(), "local Docker Unix socket required"
    old, amended = (path.read_bytes() for path in (ORIGINAL / "launch.sh", HERE / "launch.sh"))
    anchor = b' > "$stage/factorial_arm.py"\n'
    assert old.count(anchor) == 1 and amended == old.replace(
        anchor, anchor + b'chmod 0644 "$stage/factorial_arm.py"\n')
    factorial = ORIGINAL.with_name("2026-11-25-dense-fixed8-logical-factorial")
    grant = ORIGINAL.with_name("2026-11-25-dense-logical-grant-key-native")
    inputs = [*(ORIGINAL / name for name in ("overlay.py", "enqueue-stock.sql", "stage_arm.py")),
              *(factorial / name for name in ("factorial_arm.py", "render.py")),
              *(grant / name for name in ("contract.py", "stock.sql", "candidate.sql"))]
    assert "os.umask(0o077)" in (ROOT / "tools/durable-jobs-bench/worker.py").read_text()
    scratch = ROOT / "target"
    scratch.mkdir(exist_ok=True)
    cases = []
    with TemporaryDirectory(dir=scratch, prefix="dense-enqueue-mode-") as directory:
        stage = Path(directory)
        with tarfile.open(ORIGINAL / "source-deb2a3bc49fe.tar.gz") as archive:
            for name in PUBLIC_FILES:
                (stage / name).write_bytes(archive.extractfile(f"source/{PROFILE}/{name}").read())
        for path in inputs:
            shutil.copyfile(path, stage / path.name)
        (stage / "original.sh").write_bytes(old)
        (stage / "amended.sh").write_bytes(amended)
        (stage / "entry_probe.py").write_text(PROBE)
        pins = {path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted(stage.iterdir())}
        for launcher, mask, treatment, expected in (
            ("original.sh", "077", "stock", 2),
            ("original.sh", "022", "stock", 0),
            ("amended.sh", "077", "stock", 0),
            ("amended.sh", "077", "projected", 0),
        ):
            result = subprocess.run([
                docker, "--host", f"unix://{socket}", "run", "--rm", "--pull=never", "--network=none",
                "--user=0:0", "--mount", f"type=bind,src={stage},dst=/packet,readonly",
                "--entrypoint", "/bin/bash", IMAGE, "-c", BOOTSTRAP,
                "bootstrap", BASE, SOURCE, PROFILE, mask, launcher, treatment,
            ], text=True, capture_output=True, timeout=35, **options)
            assert result.returncode == expected, (launcher, result.returncode, result.stdout, result.stderr)
            row = dict(launcher=launcher, umask=mask, treatment=treatment, exit_code=expected)
            if expected == 2:
                match = re.fullmatch(r"python3: can't open file '/var/tmp/dense-enqueue-jsonb-path\."
                                     r"[A-Za-z0-9]+/factorial_arm\.py': \[Errno 13\] Permission denied\n",
                                     result.stderr)
                assert match and not result.stdout, (result.stdout, result.stderr)
                row["stderr"] = result.stderr.rstrip()
            else:
                assert not result.stderr, result.stderr
                row["entry"] = json.loads(result.stdout)
                assert row["entry"] == dict(effective_uid=65534, python_mode="0o644",
                                            disposition="stopped before product code",
                                            projected=(treatment == "projected"))
            cases.append(row)
    return dict(image=IMAGE, cases=cases, input_sha256=pins)


if __name__ == "__main__":
    print(json.dumps(verify(), indent=2, sort_keys=True))
