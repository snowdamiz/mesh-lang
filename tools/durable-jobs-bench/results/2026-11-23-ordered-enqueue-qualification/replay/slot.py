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
WIRE_STAGE = "wire-cached"
WIRE_GRANT = "NATIVE-ORDERED-WIRE-SETUP"
WIRE_TEST = "db::pg::tests::ordered_enqueue_reads_a_backpressured_window_and_retires_a_truncated_connection"
WIRE_REVISION = "77c447f773a4d0af5c5578873a2410301b3ba069"
WIRE_EXECUTABLE = "33ebb48f27c4e1623b5019400847812deb48ca6a4dba26b594efb0587a301954"
WIRE_PATH = "/var/lib/durable-jobs-bench/native/builds/mesh-ordered-enqueue-v1/mesh"
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
if [[ "$step" == wire-cached ]]; then
    test "$(uname -sm)" = 'Linux x86_64'
    test "$(cat /proc/sys/kernel/random/boot_id)" = "$EXPECTED_BOOT"
    test "$(uname -r)" = "$EXPECTED_KERNEL"
    test "$(findmnt -T "$base/scratch" -rno MAJ:MIN)" = "$EXPECTED_MOUNT"
    mkdir -m 0700 "$evidence"
    printf '%s\n' "$(uname -sm)" "$EXPECTED_BOOT" "$EXPECTED_KERNEL" "$EXPECTED_MOUNT" >"$evidence/host.txt"
    code=0
    sha256sum "$wire_path" >"$evidence/executable-before.sha256" 2>"$evidence/step.log" || code=$?
    if (( code == 0 )); then
        NATIVE_WIRE_GRANT_END=$expires timeout --signal=TERM --kill-after=5 \
            "$((expires - $(date +%s) - 20))" \
            bash -euo pipefail -c "$wire_command" >>"$evidence/step.log" 2>&1 || code=$?
    fi
    sha256sum "$wire_path" >"$evidence/executable-after.sha256" 2>>"$evidence/step.log" || code=$?
    printf '%s\n' "$code" >"$evidence/exit.txt"
    tar -czf - -C "$evidence" .
    rm -r -- "$evidence"
    exit "$code"
fi
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


def authorization(marker, prefix="NATIVE-ORDERED-ENQUEUE-SETUP"):
    match = re.fullmatch(rf"{prefix} ([A-Za-z0-9][A-Za-z0-9-]{{0,63}}) "
                         r"(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ) "
                         r"(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ)", marker)
    grants = [line for line in STATUS.read_text().splitlines()
              if line.startswith(prefix + " ")]
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


def cached_wire_command():
    """Execute the pinned proof recipe without staging a second build."""
    document = (HERE.parent / "NATIVE-WIRE-PROOF.md").read_text()
    recipe, = re.findall(r"^\x60\x60\x60bash\n(.*?)^\x60\x60\x60$", document, re.MULTILINE | re.DOTALL)
    return recipe


def verify_cached_wire(directory, write=False):
    raw = directory / "raw.tar.gz"
    if validate_archive(raw) != 0 or (directory / "ssh-exit.txt").read_text().strip() != "0":
        raise ValueError("cached wire proof must exit 0")
    request = json.loads((directory / "request.json").read_text())
    if (request.get("candidate_source_sha256"), request.get("stage"), request.get("build")) != (
        SOURCE_SHA, WIRE_STAGE, "v1"
    ):
        raise ValueError("cached wire proof request differs from the sealed build")
    if request.get("command_sha256") != hashlib.sha256(cached_wire_command().encode()).hexdigest():
        raise ValueError("cached wire proof command differs from the documented recipe")
    published_vectors = (HERE.parent / "wire-vectors.sha256").read_bytes()
    expected_hash = f"{WIRE_EXECUTABLE}  {WIRE_PATH}\n".encode()
    with tarfile.open(raw, "r:gz") as archive:
        files = {member.name.removeprefix("./"): archive.extractfile(member).read()
                 for member in archive if member.isfile()}
    if files.get("executable-before.sha256") != expected_hash or files.get("executable-after.sha256") != expected_hash:
        raise ValueError("cached wire proof executable hashes disagree")
    log = files["step.log"].decode()
    required_lines = [
        f"source_revision={WIRE_REVISION}",
        f"source_archive_sha256={SOURCE_SHA}",
        "vectors.sha256: OK",
        *(f"{line.split('  ', 1)[1]}: OK" for line in published_vectors.decode().splitlines()),
        "wire_exit=0",
        "running 1 test",
        f"test {WIRE_TEST} ... ok",
    ]
    if any(log.splitlines().count(line) != 1 for line in required_lines):
        raise ValueError("cached wire proof lacks exact source, vectors, or named test evidence")
    if len(re.findall(r"^test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured;", log, re.MULTILINE)) != 1:
        raise ValueError("cached wire proof did not execute exactly one successful test")
    if log.splitlines().count(f"{WIRE_PATH}: OK") != 2:
        raise ValueError("cached wire proof requires verified before/after executable hashes")
    baseline_host = json.loads((BASELINE / "series.json").read_text())["controls"]["host"]
    if request["expected_host"] != baseline_host or files["host.txt"].decode().splitlines() != [
        "Linux x86_64", *baseline_host
    ]:
        raise ValueError("cached wire proof host identity differs from the baseline")
    digests = {
        "test": WIRE_TEST, "source_revision": WIRE_REVISION, "executable_sha256": WIRE_EXECUTABLE,
        "vectors_sha256": hashlib.sha256(published_vectors).hexdigest(), "exit": 0,
        "archive_sha256": digest(raw), "request_sha256": digest(directory / "request.json"),
        "archive_files_sha256": {name: hashlib.sha256(data).hexdigest() for name, data in files.items()},
    }
    if (directory / "archive.sha256").read_text().strip() != digests["archive_sha256"]:
        raise ValueError("cached wire proof archive digest differs")
    receipt = directory / "verification.json"
    if write:
        with receipt.open("x") as result:
            result.write(json.dumps(digests, indent=2, sort_keys=True) + "\n")
    elif json.loads(receipt.read_text()) != digests:
        raise ValueError("cached wire proof verification digest differs")
    return digests


def step(root, build, grant, stage):
    cached = stage == WIRE_STAGE
    tag, starts, expires, host = authorization(grant, WIRE_GRANT) if cached else authorization(grant)
    cap, minimum = (480, 515) if cached else STEPS[stage]
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
    if cached:
        recipe = cached_wire_command()
        env.update(wire_path=WIRE_PATH, wire_command=recipe)
        request["command_sha256"] = hashlib.sha256(recipe.encode()).hexdigest()
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
    if cached:
        verify_cached_wire(directory, write=True)
    return directory


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--grant")
    parser.add_argument("--build-id", default="v1")
    parser.add_argument("--steps", nargs="+", choices=STEPS, default=list(STEPS))
    parser.add_argument("--plan", action="store_true")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--cached-wire", action="store_true")
    mode.add_argument("--verify-cached-wire", type=Path, metavar="RECEIPT")
    args = parser.parse_args()
    if args.verify_cached_wire:
        print(json.dumps(verify_cached_wire(args.verify_cached_wire), indent=2, sort_keys=True))
        return 0
    if args.plan and args.cached_wire:
        print(cached_wire_command(), end="")
        return 0
    if args.plan:
        print(f"Source package SHA-256 {SOURCE_SHA}; upload {len(package())} bytes")
        print("\n".join(args.steps))
        return 0
    if not args.grant or not re.fullmatch(r"v[1-9][0-9]{0,3}", args.build_id):
        parser.error("pass the published --grant and a candidate-specific build ID")
    if args.cached_wire and args.build_id != "v1":
        parser.error("the cached wire proof uses the sealed v1 build")
    tag, *_ = authorization(args.grant, WIRE_GRANT) if args.cached_wire else authorization(args.grant)
    if args.cached_wire:
        runs = LANE / "wire-followthrough"
        runs.mkdir(mode=0o700, exist_ok=True)
        with (SUITE / ".timing.lock").open("r") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            root = runs / tag
            root.mkdir(mode=0o700)
            step(root, args.build_id, args.grant, WIRE_STAGE)
        return 0
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
    except (OSError, KeyError, ValueError, subprocess.SubprocessError, tarfile.TarError) as error:
        sys.exit(str(error))
