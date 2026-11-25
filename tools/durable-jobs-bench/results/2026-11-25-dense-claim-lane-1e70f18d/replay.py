"""Verify the portable claim-lane archive using the same sealed campaign checks."""

import gzip
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
ARCHIVE = HERE / "archive"
LOCAL = ARCHIVE / "evidence/local"
BASELINE = ROOT / "tools/durable-jobs-bench/results/2026-11-24-qualified-hot-pool/source.tar.gz"
SEALED_PATH = Path("target/dense-claim-lane-1e70f18d/archive")
SOURCE = "1e70f18d624a841740d3bbb3d0fd09b269257f68"


def require(condition, detail):
    if not condition:
        raise ValueError(detail)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def read_json(path):
    return json.loads(path.read_bytes())


def verify_snapshot():
    entries = {}
    for line in (ARCHIVE / "SHA256SUMS").read_text().splitlines():
        expected, name = line.split("  ", 1)
        relative = Path(name)
        require(not relative.is_absolute() and ".." not in relative.parts and relative not in entries,
                f"invalid archive member {name}")
        entries[relative] = expected
    files = {p.relative_to(ARCHIVE) for p in ARCHIVE.rglob("*") if p.is_file()}
    require(files == entries.keys() | {Path("SHA256SUMS")}, "archive member inventory changed")
    for relative, expected in entries.items():
        require(digest((ARCHIVE / relative).read_bytes()) == expected, f"archive hash differs: {relative}")
    return len(entries)


def apply_patch(scratch):
    patch = ARCHIVE / "claim-lane.patch"
    changed = re.findall(r"^\+\+\+ b/(.+)$", patch.read_text(), re.MULTILINE)
    expected = {f"compiler/mesh-rt/src/jobs/{file}" for file in ("runner.rs", "runner_tests.rs", "trusted.rs")}
    require(len(changed) == len(expected) and set(changed) == expected, "unexpected patch scope")
    require(digest(BASELINE.read_bytes()) == "78468180f0c0be9b2b17533b54dc8cbd69e3982d84b239457b15e5beb018b530",
            "public source bundle differs")
    with tarfile.open(BASELINE) as base:
        for name in changed:
            path = scratch / name
            path.parent.mkdir(parents=True, exist_ok=True)
            with base.extractfile(f"source/{name}") as source:
                path.write_bytes(source.read())
    subprocess.run(["patch", "-p1", "-F", "0", "-t", "-i", patch], cwd=scratch,
                   capture_output=True, text=True, check=True)
    return sorted(changed)


def campaign_check(scratch):
    # Recreate the original seal's relative paths inside disposable storage.
    # The earlier pinned inputs stay in the Git checkout or export.
    for name in ("scripts", "tools"):
        (scratch / name).symlink_to(ROOT / name, target_is_directory=True)
    restored = scratch / SEALED_PATH
    shutil.copytree(ARCHIVE, restored)
    done = subprocess.run([sys.executable, "-B", restored / "campaign.py", "check"], cwd=scratch,
                          check=True, capture_output=True, text=True)
    return done.stdout.strip()


def verify_gates():
    gates = read_json(LOCAL / "gates.json")
    patch_sha = digest((ARCHIVE / "claim-lane.patch").read_bytes())
    require(gates["source"] == SOURCE and gates["patch_sha256"] == patch_sha
            and gates["untimed_cases"] == 17, "gate source or case count differs")
    counts = {}
    for gate, receipt in gates["gates"].items():
        data = gzip.decompress((LOCAL / f"{gate}.log.gz").read_bytes())
        require(receipt["exit"] == receipt["failed"] == 0 and digest(data) == receipt["raw_sha256"],
                f"gate receipt differs: {gate}")
        counts[gate] = sum(receipt["passed_per_suite"])
    require(counts == {"core-workspace": 2018, "exact-workspace-build": 0,
                       "postgres-gate": 122, "runtime-workspace": 1609}, "gate coverage differs")
    return patch_sha, counts


def verify_manifests(patch_sha):
    manifests = {name: read_json(LOCAL / f"{name}-manifest.json") for name in ("jobs", "witness", "control")}
    jobs = manifests["jobs"]
    for name, build in manifests.items():
        require(build["version"] == SOURCE and build["source_sha256"] == patch_sha
                and build["system"] == "Darwin" and build["architecture"] == "arm64"
                and build["runtime_sha256"] == jobs["runtime_sha256"]
                and build["compiler_sha256"] == jobs["compiler_sha256"], f"manifest source differs: {name}")
    for name in ("main.mpl", "writes.mpl", "worker_identity.c", "worker_identity.mpl"):
        require(digest((ARCHIVE / "overlay/fixture" / name).read_bytes()) == jobs["files"][name],
                f"canonical fixture differs: {name}")
    for mode in ("release", "kill"):
        result = read_json(LOCAL / f"semantic/teardown-{mode}/result.json")
        require(result["runner_sha256"] == manifests["witness"]["files"]["mesh"], f"teardown binary differs: {mode}")
    return {name: build["files"]["mesh"] for name, build in manifests.items()}


def verify_receipts(binaries):
    cases = list(LOCAL.rglob("result.json"))
    require(len(cases) == 17, "proof receipt inventory differs")
    require(sum(line.startswith("PASS ") for log in ("held", "edges", "control", "semantic")
                for line in (LOCAL / f"{log}.log").read_text().splitlines()) == 17, "proof case log differs")
    for relative in ("held/off", "held/on", "held/lost-ack", "held/stop", "control-proof/pause", "control-proof/scale"):
        suite, mode = relative.split("/")
        directory = LOCAL / relative
        record = read_json(directory / "result.json")
        manifest = binaries["jobs" if suite == "held" else "control"]
        require(record["executable_sha256"] == manifest
                and record["grant"]["idle"] and record["terminal"]["idle"], f"held receipt binding differs: {mode}")
        shutdown = record["shutdown"]
        require(shutdown["state"] == "stopped" and shutdown["drained"] and not shutdown["undrained_jobs"]
                and shutdown["pending_claims"] == 0, f"shutdown differs: {mode}")
        entered = mode in {"on", "lost-ack"}
        require(record["during"]["first"]["state"] == "completed"
                and record["during"]["peer"]["state"] == "executing"
                and bool(record["during"]["peer"]["attempts"]) == entered
                and record["blocked"]["finished"] == 0, f"held overlap differs: {mode}")
        if entered:
            require(len(record["overlap"]["undrained_jobs"]) == 2 and record["overlap"]["finished"] == 0,
                    f"slot charging differs: {mode}")
        if mode in {"off", "on", "lost-ack"}:
            final = record["final"]
            xids = [xid for row in final.values() for xid in (*row["attempts"], row["effect"], row["xid"])]
            require(len(xids) == len(set(xids)) == 6, f"write or terminal XIDs differ: {mode}")
        else:
            require(shutdown["outcomes"]["released_unstarted"] == 1 and not record["final"]["peer"]["attempts"],
                    f"refund differs: {mode}")
        ages = read_json(directory / "grant-ages.json")
        require(ages["dropped"] == 0 and len(ages["samples"]) == (2 if mode in {"on", "off", "lost-ack"} else 1),
                f"diagnostic coverage differs: {mode}")
    return len(cases)


def main():
    members = verify_snapshot()
    patch_sha, gates = verify_gates()
    binaries = verify_manifests(patch_sha)
    cases = verify_receipts(binaries)
    with tempfile.TemporaryDirectory(prefix=".claim-lane-replay-", dir=ROOT) as temporary:
        scratch = Path(temporary)
        patched = apply_patch(scratch / "public")
        campaign = campaign_check(scratch)
    print(json.dumps({
        "archive_members": members,
        "campaign_check": campaign,
        "local_binaries": binaries,
        "patched_paths": patched,
        "passed_gate_tests": gates,
        "proof_cases": cases,
        "setup_request_sha256": digest((ARCHIVE / "setup.json").read_bytes()),
        "timing_request_sha256": digest((ARCHIVE / "timing.json").read_bytes()),
        "stage_sha256": digest((ARCHIVE / "native-stage.tar").read_bytes()),
    }, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
