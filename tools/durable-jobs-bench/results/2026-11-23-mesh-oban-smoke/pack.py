#!/usr/bin/env python3
"""Select reproducible, credential-free members from the original SETUP receipts."""

import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import re
import tarfile

HERE = Path(__file__).resolve().parent
NATIVE = "tools/durable-jobs-bench/native"

SMOKE_FILES = {
    "application-records.json", "database.json", "failure.json", "host-before.json",
    "invocation.json", "ledger.json", "oban-broker-emulator.json", "oban-emulator.json",
    "oban-preflight.json", "oban-versions.log", "postgres-settings.json",
    "postgres-tls-sessions.json", "process-affinity.json", "row.json",
}
FAILURE_LOGS = {"broker-errors.log", "broker-output.log", "postgres.log"}

# Published evidence consists of plain text. Source snapshots are checked
# against Git separately because they intentionally contain test credentials.
SENSITIVE = re.compile(
    rb"-----BEGIN [^\r\n-]*PRIVATE KEY(?: BLOCK)?-----|"
    rb"(?i:postgres(?:ql)?://|[a-z][a-z0-9+.-]*://[^\s/<>]+@|"
    rb"\b(?:password|passwd|pgpassword|secret|cookie|api[_-]?key|access[_-]?token)"
    rb"""[\x22\x27]?\s*(?:[:=]|\x27\s+)|\b(?:create|alter)\s+(?:role|user)\b|"""
    rb"SCRAM-SHA-256\$[0-9]+:|authorization:\s*|bearer\s+[a-z0-9._-]{10,}|"
    rb"(?:gh[pousr]_|github_pat_)[a-z0-9_]{30,}|AKIA[A-Z0-9]{16})"
)
LOCAL_ARTIFACT = re.compile(
    r"(?i)(?:^|/)(?:\._[^/]+|\.git|\.env[^/]*|__pycache__|PG_VERSION|pg_wal|"
    r"pg_hba\.conf|pg_ident\.conf|id_(?:rsa|ed25519)|[^/]+\.(?:key|p12|pem|"
    r"db|sqlite3?|pyc|dump))(?:/|$)"
)


def require(ok, message):
    if not ok:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def members(data):
    """Read regular members without extracting them into the filesystem."""
    files = {}
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        for member in archive:
            path = PurePosixPath(member.name)
            name = path.as_posix()
            require(not path.is_absolute() and ".." not in path.parts,
                    f"unsafe archive path: {name}")
            if member.isdir():
                continue
            require(member.isfile() and name not in files, f"unexpected archive member: {name}")
            files[name] = archive.extractfile(member).read()
    return files


def audit(files):
    for name, data in files.items():
        require(not LOCAL_ARTIFACT.search(name) and b"\0" not in data
                and not SENSITIVE.search(data), f"disallowed setup evidence: {name}")
        data.decode("utf-8")


def audit_overlays(overlays):
    # Inspect the historical source using placeholders for these two Python expressions.
    source_for_audit = dict(overlays)
    source_for_audit["v2/arm.py"] = overlays["v2/arm.py"].replace(
        b"password=None", b"credential=None"
    ).replace(b"secret = password.encode()", b"credential = password.encode()")
    audit(source_for_audit)


def tar_bytes(files):
    buffer = io.BytesIO()
    with gzip.GzipFile(fileobj=buffer, mode="wb", mtime=0, compresslevel=9) as compressed:
        with tarfile.open(fileobj=compressed, mode="w") as archive:
            for name, data in sorted(files.items()):
                member = tarfile.TarInfo(name)
                member.size, member.mode = len(data), 0o644
                archive.addfile(member, io.BytesIO(data))
    return buffer.getvalue()


def select(root):
    smoke_dir = root / "2026-11-23"
    port_dir = root / "amd64-port-local"
    paths = {
        "native-raw": smoke_dir / "native-raw.tar.gz",
        "native-proof-logs": port_dir / "native-proof-logs.tar.gz",
        "native-wait-trace": port_dir / "native-wait-trace.tar.gz",
        **{f"source-{tag}": root / "bundles" / tag / "source.tar.gz" for tag in ("v2", "v4", "v5")},
    }
    originals = {name: path.read_bytes() for name, path in paths.items()}
    raw = members(originals["native-raw"])
    require(raw["READY.json"] == (root / "READY.json").read_bytes(), "owner READY copies differ")
    expected = {name: sha(data) for name, data in originals.items()}
    require(expected["native-raw"] == (smoke_dir / "native-raw.sha256").read_text().split()[0],
            "owner native archive digest differs")
    proof_sums = {Path(path).stem.removesuffix(".tar"): digest for digest, path in (
        line.split("  ", 1) for line in (port_dir / "SHA256SUMS").read_text().splitlines())}
    for name, digest in proof_sums.items():
        require(expected[name] == digest, f"owner {name} archive digest differs")

    retained = {}
    attempts = {}
    manifests = {}
    for path, archive in sorted(raw.items()):
        if not path.startswith("native-smoke/") or not path.endswith("/evidence.tar.gz"):
            continue
        attempt = str(PurePosixPath(path).parent)
        name = PurePosixPath(attempt).name
        status = int(raw[attempt + "/exit-status.txt"])
        checksum = sha(archive)
        require(checksum == raw[attempt + "/sha256"].decode().split()[0],
                f"owner archive checksum differs: {name}")
        for file, data in members(archive).items():
            if file in SMOKE_FILES or (status and file in FAILURE_LOGS):
                retained[f"{name}/{file}"] = data
            if file == "build.json" and status == 0:
                engine = name.split("-")[0]
                previous = manifests.setdefault(f"{engine}.json", data)
                require(previous == data, f"build manifest drift: {name}")
        for file in ("exit-status.txt", "run.log", "export.log"):
            retained[f"{name}/{file}"] = raw[attempt + "/" + file]
        attempts[name] = {"exit_status": status, "original_sha256": checksum}

    snapshots = {tag: members(originals[f"source-{tag}"]) for tag in ("v2", "v4", "v5")}
    for tag in snapshots:
        receipt = (root / "bundles" / tag / "sha256").read_text().strip()
        require(expected[f"source-{tag}"] == receipt, f"owner {tag} source digest differs")
    staged = snapshots["v5"]
    source_sums = "".join(f"{sha(data)}  {name}\n" for name, data in sorted(staged.items()))
    overlays = {"v5/README.md": staged[f"{NATIVE}/README.md"]}
    for tag in ("v2", "v4"):
        for name, data in snapshots[tag].items():
            if name.startswith(NATIVE + "/") and data != staged.get(name):
                overlays[f"{tag}/{Path(name).name}"] = data

    setup = {
        name: raw[name] for name in (
            "bootstrap-oban-v1.log", "bootstrap-mesh-v1.log", "pg17.11-native-identity.txt",
            "pg17.11-preflight-01.log", "pg17.11-preflight-02.log", "meshc-handoff-3f69b150.txt",
            "2026-11-23/native-build-identity.txt", "2026-11-23/staged-wrapper-verification.txt",
        )
    }
    # Keep the checksum and completion lines from the verbose successful install.
    setup["oban-v2-service-sha256.txt"] = b"".join(
        line for line in raw["bootstrap-oban-v2.log"].splitlines(keepends=True)
        if re.match(rb"[0-9a-f]{64}  /var/lib/durable-jobs-bench/native/toolchain/", line)
        or line.startswith(b"Build ready at ")
    )
    for name in ("setup-after-port-proof-01.log", "setup-after-port-proof-02.log"):
        setup[name] = (root / name).read_bytes()
    for name in ("baseline-platform.log", "cleanup.log", "prove-on-host-final.sh", "prove-on-host-lto0.sh"):
        setup["amd64/" + name] = (port_dir / name).read_bytes()

    for name in ("native-proof-logs", "native-wait-trace"):
        audit(members(originals[name]))
    exact_wait = (port_dir / "exact-wait.txt").read_bytes()
    audit(retained | setup | manifests | {"exact-wait.txt": exact_wait, "READY.json": raw["READY.json"]})
    audit_overlays(overlays)
    metadata = {
        "original_archives": expected, "attempts": attempts,
        "retained_smoke_files": {name: sha(data) for name, data in sorted(retained.items())},
        "original_rows": json.loads(raw["2026-11-23/native-functional-rows.json"]),
    }
    for row in metadata["original_rows"]:
        row["evidence"] = Path(row["evidence"]).parent.name + "/evidence.tar.gz"
    return {
        **manifests, "READY.json": raw["READY.json"], "source-v5.sha256": source_sums.encode(),
        "exact-wait.txt": exact_wait,
        "native-proof-logs.tar.gz": originals["native-proof-logs"],
        "native-wait-trace.tar.gz": originals["native-wait-trace"],
        "native-smokes.tar.gz": tar_bytes(retained),
        "setup-history.tar.gz": tar_bytes(setup),
        "staged-source-overlays.tar.gz": tar_bytes(overlays),
        "originals.json": (json.dumps(metadata, indent=2, sort_keys=True) + "\n").encode(),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("originals", type=Path, help="owner's target/linode-jobs-native directory")
    parser.add_argument("--out", type=Path, help="write the selected evidence to a fresh directory")
    args = parser.parse_args()
    files = select(args.originals)
    if args.out:
        args.out.mkdir()
        for name, data in files.items():
            (args.out / name).write_bytes(data)
    else:
        for name, data in files.items():
            require((HERE / name).read_bytes() == data, f"repack differs: {name}")
        print(f"Verified {len(files)} files against the original SETUP evidence")


if __name__ == "__main__":
    main()
