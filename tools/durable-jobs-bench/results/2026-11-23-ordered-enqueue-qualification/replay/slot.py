#!/usr/bin/env python3
"""Execute exclusive ordered-enqueue SETUP stages under a published reservation."""
import argparse
from datetime import datetime, timezone
import fcntl
import hashlib
import gzip
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import shlex
import shutil
import subprocess
import sys
import tarfile
import time
from uuid import uuid4

HERE = Path(__file__).resolve().parent
ROOT = next(path for path in HERE.parents if (path / "tools/durable-jobs-bench/run.py").is_file())
LANE = ROOT / "target/linode-ordered-enqueue"
SUITE = ROOT / "tools/durable-jobs-bench"
BASELINE = SUITE / "results/2026-11-23-baseline"
STATUS = LANE.parent / "linode-jobs-native/amd64-port-status.md"
SOURCE = LANE / "native-bundle-v1/source.tar.gz"
SOURCE_SHA = "0c1695ebb875c875095429750731c137656e47b04c59e273d03b813392c5caef"
sys.path.insert(0, str(SUITE))
# Use the controller's exact host key, identity, and transport settings.
from run import SSH, digest


STEPS = {
    "build": (540, 240),
    "semantic-stock-dense": (90, 20),
    **{f"semantic-candidate-{profile}": (90, 20) for profile in ("dense", "short", "sparse", "large", "slow")},
    "producer-faults": (50, 20),
    "producer-component": (75, 20),
    "producer-dense": (75, 20),
    "producer-large": (110, 25),
    "producer-sparse": (75, 20),
    **{f"producer-wire-{profile}": (50, 20) for profile in ("component", "dense", "large", "sparse")},
    "wire-test": (180, 55),
}

REMOTE = r"""
set -euo pipefail
umask 077
test "$(date +%s)" -ge "$starts"
test "$(date +%s)" -lt "$expires"
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9</var/lib/durable-jobs-bench/native/native.lock
flock -n 9
base=/var/lib/durable-jobs-bench/native
setup="$base/setup/ordered-enqueue-$build"
if [[ "$step" == build ]]; then
    mkdir -p "$base/setup"
    mkdir -m 0711 "$setup"
    mkdir -m 0755 "$setup/input"
fi
test -d "$setup" && test ! -L "$setup"
evidence="$setup/evidence-$receipt"
mkdir -m 0700 "$evidence"
code=0
if [[ "$step" == build ]]; then
    timeout 25 tar -xf - -C "$setup/input" --no-same-owner >"$evidence/upload.log" 2>&1 || code=$?
    chmod 0644 "$setup/input/"*.py
fi
budget=$((expires - $(date +%s) - 32))
if (( budget > max_stage )); then budget=$max_stage; fi
if (( code == 0 && budget >= min_stage )); then
    timeout --signal=TERM --kill-after=12 "$budget"       bash "$setup/input/remote.sh" "$step" "$evidence" "$build" >"$evidence/step.log" 2>&1 || code=$?
elif (( code == 0 )); then
    printf 'Insufficient reservation time after acquiring both locks.\n' >"$evidence/step.log"
    code=75
fi
printf '%s\n' "$code" >"$evidence/exit.txt"
# No scratch, certificates, keys, connection environments, or core dumps are
# exported. The runners redact their guaranteed-disposable credentials.
tar -czf - -C "$evidence" .
exit "$code"
"""


def authorization(marker):
    match = re.fullmatch(r"NATIVE-ORDERED-ENQUEUE-SETUP ([A-Za-z0-9][A-Za-z0-9-]{0,63}) "
                         r"(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ) "
                         r"(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ)", marker)
    grants = [line for line in STATUS.read_text().splitlines()
              if line.startswith("NATIVE-ORDERED-ENQUEUE-SETUP ")]
    if not match or not grants or grants[-1] != marker:
        raise ValueError("the matrix owner must publish the current exact grant")
    tag, begin, end = match.groups()
    begin, end = [int(datetime.strptime(item, "%Y-%m-%dT%H:%M:%SZ")
                      .replace(tzinfo=timezone.utc).timestamp()) for item in (begin, end)]
    if not begin <= time.time() < end:
        raise ValueError("the published grant is inactive")
    index = json.loads((BASELINE / "index.json").read_text())
    baseline = json.loads((BASELINE / "series.json").read_text())
    expected = {f"{b['group']:02d}-{b['profile']}" for b in baseline["plan"]}
    if len(expected) != 25 or set(index["validated_blocks"]) != expected or index["missing_blocks"]:
        raise ValueError("the official baseline has not sealed all 25 blocks")
    if baseline["inventory"]["cpus"] != [0, 1]:
        raise ValueError("the baseline CPU affinity has changed")
    return tag, begin, end, baseline["controls"]["host"]


def package():
    if digest(SOURCE) != SOURCE_SHA:
        raise ValueError("sealed candidate source SHA-256 changed")
    blob = io.BytesIO()
    with tarfile.open(fileobj=blob, mode="w") as archive:
        archive.add(SOURCE, arcname="source.tar.gz")
        for name in ("remote.sh", "producer.py", "producer_protocol.py"):
            archive.add(HERE / name, arcname=name)
    return blob.getvalue()


def validate_archive(path):
    with gzip.open(path, "rb") as payload, open(os.devnull, "wb") as sink:
        shutil.copyfileobj(payload, sink)
    with tarfile.open(path, "r:gz") as archive:
        names = set()
        for member in archive:
            normalized = PurePosixPath(member.name)
            if normalized.is_absolute() or ".." in normalized.parts or normalized in names:
                raise ValueError(f"unexpected retained evidence entry {member.name}")
            names.add(normalized)
            if member.isdir():
                continue
            if not member.isfile() or normalized.suffix not in {".log", ".json", ".txt", ".sha256"}:
                raise ValueError(f"unexpected retained evidence file {member.name}")
            data = archive.extractfile(member).read()
            if len(data) != member.size:
                raise ValueError(f"incomplete evidence file {member.name}")
            if re.search(rb"(?:postgres(?:ql)?://[^\s/:]+:(?!<redacted>)[^\s@]+@"
                         rb"|-----BEGIN [A-Z ]*PRIVATE KEY-----)", data):
                raise ValueError(f"unsanitized credential in {member.name}; keep local and private")
        return int(archive.extractfile("./exit.txt").read())


def step(root, build, grant, stage):
    tag, starts, expires, host = authorization(grant)
    cap, minimum = STEPS[stage]
    if expires - time.time() <= minimum + 34:
        raise ValueError(f"insufficient grant time to begin {stage}")
    receipt = f"{stage}-{uuid4().hex}"
    directory = root / receipt
    directory.mkdir(mode=0o700)
    env = dict(zip(("EXPECTED_BOOT", "EXPECTED_KERNEL", "EXPECTED_MOUNT"), host, strict=True))
    env.update(starts=starts, expires=expires, step=stage, build=build, receipt=receipt,
               max_stage=cap, min_stage=minimum)
    request = {"grant": grant, "reservation": tag, "build": build, "stage": stage, "receipt": receipt,
               "candidate_source_sha256": SOURCE_SHA, "expected_host": host}
    (directory / "request.json").write_text(json.dumps(request, indent=2) + "\n")
    payload = package() if stage == "build" else None
    if payload:
        (directory / "upload.sha256").write_text(hashlib.sha256(payload).hexdigest() + "\n")
    arguments = " ".join(f"{key}={shlex.quote(str(value))}" for key, value in env.items())
    remote = f"{arguments} bash -c {shlex.quote(REMOTE)}"
    partial = directory / "remote.partial"
    with partial.open("xb") as out, (directory / "ssh.log").open("xb") as err:
        ssh = subprocess.run([*SSH, remote], input=payload if payload is not None else b"",
                             stdout=out, stderr=err, timeout=max(1, expires - time.time()))
    (directory / "ssh-exit.txt").write_text(str(ssh.returncode) + "\n")
    if partial.stat().st_size == 0:
        raise ValueError(f"{stage} returned no archive, SSH exit {ssh.returncode}. Receipt: {directory}")
    code = validate_archive(partial)
    if ssh.returncode != code:
        raise ValueError(f"{stage} archive exit {code} differs from SSH exit {ssh.returncode}. Receipt: {directory}")
    partial.rename(directory / "raw.tar.gz")
    (directory / "archive.sha256").write_text(digest(directory / "raw.tar.gz") + "\n")
    print(f"{stage} exit={code} {directory}", flush=True)
    if code:
        raise ValueError(f"{stage} failed; preserved {directory}")
    return directory


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--grant")
    parser.add_argument("--build-id", default="v1")
    parser.add_argument("--steps", nargs="+", choices=STEPS, default=list(STEPS))
    parser.add_argument("--plan", action="store_true")
    args = parser.parse_args()
    if args.plan:
        print(f"Source package SHA-256 {SOURCE_SHA}; upload {len(package())} bytes")
        print("\n".join(args.steps))
        return 0
    if not args.grant or not re.fullmatch(r"v[1-9][0-9]{0,3}", args.build_id):
        parser.error("pass the published --grant and a candidate-specific build ID")
    tag, *_ = authorization(args.grant)
    runs = HERE / "native-runs"
    runs.mkdir(mode=0o700, exist_ok=True)
    root = runs / f"{tag}-{uuid4().hex}"
    root.mkdir(mode=0o700)
    with (SUITE / ".timing.lock").open("r") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        for stage in args.steps:
            step(root, args.build_id, args.grant, stage)
    return 0


if __name__ == "__main__":
    os.umask(0o077)
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.SubprocessError, tarfile.TarError) as error:
        sys.exit(str(error))
