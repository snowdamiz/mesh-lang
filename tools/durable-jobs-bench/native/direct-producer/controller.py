#!/usr/bin/env python3
"""Stage the direct producer under a controller SETUP grant and freeze lane inventories."""

import argparse
import json
from pathlib import Path
import shlex
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[1]))
import run

REVISION = "3828f20d2d6beb683868cce33e6dd1958365914b"
BASE = "/var/lib/durable-jobs-bench/native"
SOURCE = f"{BASE}/source-stock-3828f20d"
STOCK = f"{BASE}/builds/mesh-stock-3828f20d"
DIRECT = f"{BASE}/builds/mesh-direct-producer-v3"
ADAPTER = f"{SOURCE}/tools/durable-jobs-bench/direct-adapter-v3"
PREPARE = HERE / "prepare.py"
PREFIX = "2026-11-23-direct-producer-v3"
PROOF_FILES = [
    "scripts/jobs-parity.py", "scripts/jobs-parity-profiles.json",
    *(f"proof/jobs-parity/linux-regression/{name}" for name in
      ("trial.py", "proof_transport.py", "oban_runtime.py", "application.sql", "fence.sql")),
]


def setup(evidence):
    evidence.mkdir(exist_ok=True)
    remote_command = f"""
set -euo pipefail
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<{BASE}/native.lock
flock -n 9

umask 022
export PATH={BASE}/toolchain/llvm-21.1.8/bin:/usr/bin:/bin
export TMPDIR={BASE}/tmp LC_ALL=C.UTF-8
script=$(mktemp {BASE}/tmp/direct-producer-v3.XXXXXX)
trap 'rm -f -- "$script"' EXIT
cat > "$script"
printf '%s  %s\n' {run.digest(PREPARE)} "$script" | sha256sum -c >&2
abi={STOCK}/abi
if [[ ! -x "$abi/meshc" ]]; then abi={BASE}/cargo/release; fi
if [[ ! -f {DIRECT}/mesh.json ]]; then
    python3 -B "$script" {SOURCE} {STOCK} "$abi/meshc" "$abi/libmesh_rt.a" {DIRECT} {ADAPTER}
fi
cat /proc/sys/kernel/random/boot_id
uname -r
findmnt -no MAJ:MIN --target {BASE}
getconf CLK_TCK
{{
    sha256sum {STOCK}/mesh {STOCK}/mesh.json {DIRECT}/broker {DIRECT}/mesh.json
    sha256sum {SOURCE}/tools/durable-jobs-bench/native/{{arm.py,ledger.py,run}}
    sha256sum {ADAPTER}/{{arm.py,direct-trial.py,ledger.py,run}}
    cd {SOURCE}
    sha256sum {' '.join(PROOF_FILES)} | sed 's@  @  {SOURCE}/@'
}}
"""
    # The outer local lock covers the whole native staging and identity check.
    import fcntl
    with (run.SUITE / ".timing.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        with PREPARE.open("rb") as source, (evidence / "setup.log").open("xb") as stderr:
            result = run.ssh(["bash", "-c", remote_command], stdin=source, stdout=run.subprocess.PIPE,
                             stderr=stderr, timeout=180)
            result.check_returncode()
        (evidence / "native-identity.txt").write_bytes(result.stdout)
        for name, path in (("stock-manifest.json", f"{STOCK}/mesh.json"),
                           ("direct-manifest.json", f"{DIRECT}/mesh.json"),
                           ("broker-link.log", f"{DIRECT}/broker-link.log"),
                           ("broker-source.mpl", f"{DIRECT}/broker-source/main.mpl")):
            command = f"""
set -euo pipefail
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9<{BASE}/native.lock
flock -n 9
cat {shlex.quote(path)}
"""
            with (evidence / name).open("xb") as output:
                run.ssh(["bash", "-c", command], stdout=output, timeout=20).check_returncode()
    inventories(evidence)


def inventories(evidence):
    boot, kernel, disk, ticks, *hashes = (
        line.strip() for line in (evidence / "native-identity.txt").read_text().splitlines()
    )
    verified = {path: checksum for checksum, path in (line.split(maxsplit=1) for line in hashes)}
    for name, path in (("stock-manifest.json", f"{STOCK}/mesh.json"),
                       ("direct-manifest.json", f"{DIRECT}/mesh.json")):
        run.require(run.digest(evidence / name) == verified[path], f"wrong exported {name}")
    stock = run.read_json(evidence / "stock-manifest.json")
    direct = run.read_json(evidence / "direct-manifest.json")
    run.require(stock["version"] == direct["version"] == REVISION, "wrong stock compiler source")
    run.require((direct["stock_manifest"], direct["stock_manifest_sha256"], direct["stock_runner_sha256"],
                 stock["files"]["mesh"], direct["files"]["broker"])
                == (f"{STOCK}/mesh.json", verified[f"{STOCK}/mesh.json"], verified[f"{STOCK}/mesh"],
                    verified[f"{STOCK}/mesh"], verified[f"{DIRECT}/broker"]), "wrong worker or broker binding")
    run.require(run.digest(evidence / "broker-source.mpl") == direct["files"]["broker-source/main.mpl"],
                "exported broker source changed")
    original = run.read_json(run.RESULTS / "2026-11-23-baseline/inventory.json")
    engines = original["engines"]

    def mesh_spec(directory, artifact):
        required = {f"{STOCK}/mesh", f"{STOCK}/mesh.json", f"{artifact}/mesh.json",
                    *(f"{SOURCE}/{path}" for path in PROOF_FILES),
                    *(f"{directory}/{name}" for name in ("arm.py", "ledger.py", "run"))}
        if artifact == DIRECT:
            required.update((f"{ADAPTER}/direct-trial.py", f"{DIRECT}/broker"))
            run.require(all(verified[f"{ADAPTER}/{name}"] == sha
                            for name, sha in direct["adapter_files"].items()), "wrong direct adapter")
        return dict(engines["mesh"], argv=["/usr/bin/env", f"NATIVE_JOBS_ARTIFACTS={artifact}",
                                          f"{directory}/run", "mesh"],
                    version=REVISION, artifact_sha256=verified[f"{artifact}/mesh.json"],
                    verified_files={path: verified[path] for path in sorted(required)})

    classic = mesh_spec(f"{SOURCE}/tools/durable-jobs-bench/native", STOCK)
    candidate = mesh_spec(ADAPTER, DIRECT)

    def lane(engine, specification):
        return dict(engine=engine, inventory=dict(cpus=original["cpus"],
                                                   engines={**engines, engine: specification}))

    lanes = {"mesh-stock-a": lane("mesh", classic), "mesh-stock-b": lane("mesh", classic),
             "mesh-direct-file": lane("mesh", candidate), "oban": lane("oban", engines["oban"]),
             "river": lane("go", engines["go"]), "apalis": lane("rust", engines["rust"])}
    for name, profiles in (("large", ["large"]), ("controls", ["noop", "short", "sparse"])):
        config = dict(contract="strict-frozen", profiles=profiles, cpus=original["cpus"],
                      host=[boot, kernel, disk], clock_ticks=int(ticks), lanes=lanes,
                      description="Direct-file producer adapter only. Same exact stock Mesh runner path "
                      "in A/A/B. Original stock peer builds, frozen job data, PG caps and timing windows.")
        for lane_spec in lanes.values():
            run.check_inventory(lane_spec["inventory"])
        # Lane insertion order is part of the timing plan.
        with (evidence / f"{name}.json").open("x") as stream:
            json.dump(config, stream, indent=2, allow_nan=False)
            stream.write("\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("setup", "inventories"))
    parser.add_argument("evidence", type=Path)
    args = parser.parse_args()
    (setup if args.action == "setup" else inventories)(args.evidence)
