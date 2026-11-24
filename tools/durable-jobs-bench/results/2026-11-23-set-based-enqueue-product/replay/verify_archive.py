"""Verify the integrated producer and its preserved evidence without a host reservation."""

import gzip
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tarfile


PACKET = Path(__file__).resolve().parent.parent
ROOT = PACKET.parents[3]
SHARED = "5dfca3d943db02a9a9ea508c26231fea3d94a79f"
SHORT = (PACKET.parent / "2026-11-23-bulk-enqueue-v2-short"
         / "evidence/bulk-enqueue-v2-short")


def require(condition, subject):
    if not condition:
        raise ValueError(f"verification failed for {subject}")


def manifest(path):
    for line in path.read_text().splitlines():
        digest, name = line.split(maxsplit=1)
        yield name, digest


def verify():
    local = PACKET / "evidence/local"
    request = json.loads((PACKET / "request.json").read_bytes())
    for name, digest in request["pins"].items():
        require(hashlib.sha256((ROOT / name).read_bytes()).hexdigest() == digest, name)

    archive = ROOT / request["source_archive"]
    tree = subprocess.check_output(["git", "ls-tree", "-r", "-z", SHARED], cwd=ROOT)
    blobs = {}
    for entry in tree.rstrip(b"\0").split(b"\0"):
        metadata, path = entry.split(b"\t", 1)
        blobs[path.decode()] = metadata.split()[2].decode()
    with tarfile.open(archive) as tar:
        matched = 0
        for member in tar:
            if member.isdir():
                continue
            require(member.isfile(), member.name)
            data = tar.extractfile(member).read()
            git_blob = b"blob %d\0" % len(data) + data
            require(hashlib.sha1(git_blob).hexdigest() == blobs.get(member.name), member.name)
            matched += 1
    require(matched == 1200, "source archive member count")
    for name, digest in manifest(local / "gated-source.sha256"):
        blob = subprocess.check_output(["git", "cat-file", "blob", blobs[name]], cwd=ROOT)
        require(hashlib.sha256(blob).hexdigest() == digest, name)

    counts = {}
    for name, expected in manifest(local / "v2-short-raw.sha256"):
        path = SHORT / name
        if name.endswith(".log"):
            data = gzip.decompress(path.with_suffix(".log.gz").read_bytes())
        else:
            data = path.read_bytes()
        require(hashlib.sha256(data).hexdigest() == expected, name)
        counts[name] = expected
    require(len(counts) == 187, "V2 short manifest")
    gates = ["rustfmt", "workspace-build", "workspace-tests", "diagnostics-minimal",
             "effect-transaction-17.11", "pg-17.11-attempt1-forged-commit-tag"]
    for version in ("17.11", "18.7", "19.0"):
        gates.extend((f"pg-{version}", f"effect-fence-{version}"))
    for gate in gates:
        expected = 1 if gate == "pg-17.11-attempt1-forged-commit-tag" else 0
        require(int((local / f"{gate}.exit").read_text()) == expected, gate)
        gzip.decompress((local / f"{gate}.log.gz").read_bytes())
    tests = gzip.decompress((local / "workspace-tests.log.gz").read_bytes())
    total = sum(map(int, re.findall(rb"test result: ok\. (\d+) passed", tests)))
    require(total == 4821, "workspace test count")
    shared = PACKET / "evidence/shared"
    require(int((shared / "pg-17.11.exit").read_text()) == 0, "shared PostgreSQL run")
    log = gzip.decompress((shared / "pg-17.11.log.gz").read_bytes())
    require(hashlib.sha256(log).hexdigest() ==
            "c54e602aa3054e82bab53b4d513bde00ed4103a6875ce40dc904cd79b3494ff4",
            "shared PostgreSQL log")
    for schema in (b"fresh", b"upgrade"):
        require(b"Scoped Jobs role proof passed on jobs_role_" + schema in log, schema)
    require(len(set(re.findall(rb"^Jobs set-based enqueue wire (\w+) passed$", log, re.M))) == 7,
            "shared enqueue relay cases")
    checkout = re.search(rb"^Jobs proof checkout: ([0-9a-f]{40})$", log, re.M)
    require(checkout, "shared checkout")
    subprocess.run(["git", "diff", "--exit-code", "--quiet", SHARED, checkout[1].decode(),
                    "--", ".", ":!tools/durable-jobs-bench"], cwd=ROOT, check=True)
    print(f"Verified {matched} source files, {len(counts)} native evidence files and {total} workspace tests.")


if __name__ == "__main__":
    verify()
