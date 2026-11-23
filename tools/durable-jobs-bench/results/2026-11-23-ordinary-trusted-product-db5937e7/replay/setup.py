#!/usr/bin/env python3
"""Qualify the exact product runtime under a new controller SETUP assignment."""

import argparse
from copy import deepcopy
from datetime import datetime, timedelta, timezone
import fcntl
from pathlib import Path
import re
import sys
import tarfile
from uuid import uuid4

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
REQUESTS = ROOT / "target/linode-jobs-stepchange/requests"
sys.path.insert(0, str(ROOT / "tools/durable-jobs-bench"))
import run

REQUEST = "ordinary-trusted-product-db5937e7"
REVISION = "db5937e7e8988f935e9be16b68a2561c2250cc27"
BASE = "/var/lib/durable-jobs-bench/native"
CONTRACT = "ordinary-trusted-shared-process"
PROFILES = {"short": 128, "noop": 1024}
CPUS = [0, 1]


def check_request(path):
    request = run.read_json(path)
    if request["id"] != REQUEST or request["source_sha"] != REVISION or request["state"] != "ready":
        raise ValueError("wrong request or source revision")
    pins = {
        HERE / "setup.py": request["setup_driver_sha256"],
        HERE / "native-setup.sh": request["native_setup_sha256"],
        Path(request["source_archive"]): request["source_archive_sha256"],
        Path(request["peer_inventory"]): request["peer_inventory_sha256"],
    }
    pins.update({ROOT / path: sha for path, sha in request["broker_drivers"].items()})
    for file, expected in pins.items():
        run.require(run.digest(file) == expected, f"SHA-256 mismatch: {file}")
    peer = run.read_json(Path(request["peer_inventory"]))
    run.check_inventory(peer["inventory"])
    run.require(peer["inventory"]["cpus"] == CPUS and peer["execution_contract"] == CONTRACT,
                "frozen V1 has a different CPU or execution contract")
    with tarfile.open(request["source_archive"], "r:gz") as bundle:
        with bundle.extractfile("native-validation/revision") as text:
            run.require(text.read().decode().strip() == REVISION, "wrong revision inside archive")
    return request, peer


def confirm_grant():
    status = (REQUESTS / "STATUS.md").read_text()
    grants = re.findall(r"^(SETUP|TIMING|RELEASED) ([\w-]+) (\S+)(?: (\S+))?", status, re.M)
    if not grants:
        raise ValueError("no controller assignment")
    mode, request, start, end = grants[-1]
    if (mode, request) != ("SETUP", REQUEST):
        raise ValueError(f"await a new controller SETUP assignment for {REQUEST}")
    now = datetime.now(timezone.utc)
    if not datetime.fromisoformat(start) <= now <= datetime.fromisoformat(end) - timedelta(minutes=14):
        raise ValueError("the new assignment must have at least 14 minutes remaining")


def qualify(out, peer, native_root, build):
    with tarfile.open(out / "evidence.tar.gz", "r:gz") as archive:
        def get(profile, lane, name):
            return run.file_json(archive, f"./{profile}-{lane}/{name}")

        with archive.extractfile("./setup-identity.txt") as text:
            host, boot, mount, ticks, *_ = text.read().decode().splitlines()
        run.require(host.startswith("Linux ") and host.endswith(" x86_64"), "wrong native host")
        with archive.extractfile("./artifacts.txt") as text:
            hashes = dict((name, sha) for sha, name in
                          (line.split() for line in text.read().decode().splitlines()))
        frozen_spec = peer["inventory"]["engines"]["mesh"]
        control_sha = frozen_spec["artifact_sha256"]
        product_sha = hashes[f"{build}/mesh.json"]
        run.require(product_sha != control_sha, "candidate manifest equals frozen V1")
        paired = {}
        for profile, completed in PROFILES.items():
            pair = {}
            for label, version, sha in (("frozen-v1", frozen_spec["version"], control_sha),
                                        ("product", REVISION, product_sha)):
                row = get(profile, label, "row.json")
                ledger = get(profile, label, "ledger.json")
                manifest = get(profile, label, "build.json")
                db = get(profile, label, "postgres-settings.json")
                masks = get(profile, label, "process-affinity.json")
                run.require(not ledger["local_smoke"] and ledger["phase"] == "diagnostic"
                            and ledger["execution_contract"] == CONTRACT, "wrong proof phase")
                run.require(row["artifact_sha256"] == sha and manifest["version"] == version,
                            f"{label}: wrong executable manifest")
                run.require(len(masks) == run.PROFILES[profile][0] + 1 and
                            all(mask == CPUS for mask in masks.values()), "wrong native CPU affinity")
                report = run.reconcile(ledger, "mesh", profile, {
                    "version": version, "artifact_sha256": sha,
                })
                run.require(report["completed"] == completed and row["committed_work_records"] == 2 * completed,
                            "incomplete ledger")
                run.require(db["version"] == "17.11" and
                            all(db[key] for key in ("tls", "fsync", "full_page_writes",
                                                   "synchronous_commit", "track_commit_timestamp")),
                            "PostgreSQL durability, TLS, or version differs")
                pair[label] = dict(input=ledger["input_sha256"], database=db, manifest=manifest,
                                   runners=ledger["runners"], slots=ledger["slots"])
            old, new = pair["frozen-v1"], pair["product"]
            run.require(all(old[key] == new[key] for key in ("input", "database", "runners", "slots")),
                        f"{profile}: matched inputs, PostgreSQL settings, or slots differ")
            old_files, new_files = old["manifest"]["files"], new["manifest"]["files"]
            run.require(old_files.keys() == new_files.keys()
                        and all(old_files[name] == new_files[name] for name in old_files if name != "mesh")
                        and old["manifest"]["compiler_sha256"] == new["manifest"]["compiler_sha256"]
                        and old_files["mesh"] != new_files["mesh"] == hashes[f"{build}/mesh"],
                        "changed fixture or compiler, or wrong candidate ELF")
            paired[profile] = dict(jobs=completed, **{key: new[key] for key in (
                "input", "database", "runners", "slots",
            )})
        faults = run.file_json(archive, "./faults/cases/cases.json")
        run.require(faults["clean"] and len(faults["cases"]) == 16 and
                    faults["cases"]["late-effect"]["late_effect_committed"] and
                    faults["cases"]["stale-token-replay"] == "rejected while replacement token was live",
                    "incomplete native fault replay")
    run.write_json(out / "qualification.json", dict(revision=REVISION, paired=paired,
                                                    fault_cases=sorted(faults["cases"]),
                                                    native_evidence=native_root))
    product = deepcopy(peer)
    product_spec = product["inventory"]["engines"]["mesh"]
    product_spec.update(
        version=REVISION, artifact_sha256=product_sha,
        argv=[*frozen_spec["argv"][:1], f"NATIVE_JOBS_ARTIFACTS={build}", *frozen_spec["argv"][2:]],
        verified_files={
            **{path: sha for path, sha in frozen_spec["verified_files"].items()
               if not path.startswith(frozen_spec["argv"][1].split("=", 1)[1] + "/")},
            **{path: hashes[path] for path in (f"{build}/mesh", f"{build}/mesh.json")},
        },
    )
    run.write_json(out / "qualified-pair.json", dict(
        contract=CONTRACT, description="Trusted Jobs product and frozen V1 on the shared native broker",
        cpus=CPUS, host=[boot, host.split()[1], mount], clock_ticks=int(ticks),
        profiles=list(PROFILES), lanes={"frozen-v1": peer, "product": product},
    ))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("request", type=Path)
    parser.add_argument("--check", action="store_true", help="verify local pins without host access")
    args = parser.parse_args()
    request, peer = check_request(args.request)
    if args.check:
        print(f"{REQUEST}: local request pins verified")
        return
    attempt = f"{REQUEST}-{uuid4().hex[:12]}"
    remote = f"{BASE}/runs/{attempt}"
    build = f"{BASE}/builds/{attempt}"
    out = Path(request["evidence_destination"]) / attempt
    with (run.SUITE / ".timing.lock").open("r") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        confirm_grant()
        out.mkdir(parents=True)
        run.write_json(out / "request.json", dict(request=request, attempt=attempt, started=run.utc_now()))
        result = None
        export = None
        try:
            with Path(request["source_archive"]).open("rb") as bundle, (out / "setup.log").open("xb") as log:
                result = run.ssh([
                    "timeout", "--kill-after=10s", "690s", "bash", "-c",
                    (HERE / "native-setup.sh").read_text(), "--",
                    request["source_archive_sha256"], attempt,
                ], stdin=bundle, stdout=log, stderr=log, timeout=710)
        finally:
            try:
                with (out / "evidence.tar.gz").open("xb") as archive, (out / "export.log").open("xb") as log:
                    export = run.ssh(["tar", "-czf", "-", "-C", remote, "."],
                                     stdout=archive, stderr=log, timeout=90)
            finally:
                run.write_json(out / "status.json", dict(
                    setup=None if result is None else result.returncode,
                    export=None if export is None else export.returncode, finished=run.utc_now(),
                ))
        result.check_returncode()
        export.check_returncode()
        qualify(out, peer, remote, build)
    print(out / "qualified-pair.json")


if __name__ == "__main__":
    main()
