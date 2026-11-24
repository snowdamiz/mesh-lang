"""Corrected short16 fused-refill launcher: the controller's sealed launcher plus the closure amendment.

Run from the checkout root by the sole controller, exactly like
target/linode-jobs-stepchange/short16-fused-refill-slots.py. That file is loaded
unchanged (pinned) for intake, the original seven-cut SETUP, its verification,
replay and receipts. The amendment adds only:

1. a build-free overlay SETUP (closure-retry/setup.sh) after the original SETUP
   qualifies, with two real native diagnostic short ledgers;
2. a rebinding of every mesh lane to the sibling replay-closure-retry adapter
   (the candidate overlay from step 1, and the already-qualified public
   b3557273 overlay for public-group);
3. TIMING through closure-retry/controller-timing.py under grant <ID>-closure-retry.

The eight lanes, the frozen five-group plan, and the original pilot gate are unchanged.
"""
import argparse
from datetime import datetime, timedelta, timezone
from collections import Counter
import fcntl
import hashlib
import json
from pathlib import Path
import runpy
import shlex
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[4]
STATE = ROOT / "target/linode-jobs-stepchange"
QUEUE = STATE / "requests"
LAUNCHER = STATE / "short16-fused-refill-slots.py"
LAUNCHER_SHA = "fc53248bc2463cd148a69f96ae8d459017d26ef9ad7036d9c9ad69cb1fe956e4"
ID = "2026-11-24-short16-fused-refill"
OID = f"{ID}-closure-retry"
ATTEMPT = "short16-fused-refill-c4c5cc2243b6"
NATIVE = Path("/var/lib/durable-jobs-bench/native")
NAMES = ("native-run", "product_arm.py", "identity.py", "backend.py")
PACKET_REL = Path("tools/durable-jobs-bench/results") / ID
PRIVATE_REPLAY = ROOT / "target/short16-fused-refill/source" / PACKET_REL / "replay"
CANDIDATE_OLD = NATIVE / f"source-{ATTEMPT}/source" / PACKET_REL / "replay"
CANDIDATE_NEW = CANDIDATE_OLD.with_name("replay-closure-retry")
REMOTE = NATIVE / f"runs/{ATTEMPT}-closure-retry"
PUBLIC_ID = "2026-11-24-trusted-jobs-product-composition"
PUBLIC_ATTEMPT = "trusted-jobs-product-623c46b92cc5"
PUBLIC_REL = Path("tools/durable-jobs-bench/results") / PUBLIC_ID
PUBLIC_OLD = NATIVE / f"source-{PUBLIC_ATTEMPT}/source" / PUBLIC_REL / "replay"
PUBLIC_NEW = PUBLIC_OLD.with_name("replay-closure-retry")
PUBLIC_REPLAY = ROOT / "target" / PUBLIC_ID / "source" / PUBLIC_REL / "replay"
PUBLIC_QUALIFICATION = ROOT / PUBLIC_REL / f"evidence/native/setup/{PUBLIC_ATTEMPT}-closure-retry/qualification.json"
PUBLIC_RELEASE = QUEUE / f"{PUBLIC_ID}-closure-retry.released.json"
PUBLIC_PINS = {
    PUBLIC_QUALIFICATION: "590a3dc95cf10b3f4ce2586f73de620f90a29d30af4909b986fa6a8b3797d5c0",
    PUBLIC_RELEASE: "091c1d7382754929124dac432ad9f50d481b669a2f3736e2572dc044a184fbf5",
}
DRIVER = HERE / "controller-timing.py"


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def load_original():
    if digest(LAUNCHER) != LAUNCHER_SHA:
        raise ValueError("controller's sealed fused launcher changed")
    return runpy.run_path(str(LAUNCHER))


base = load_original()
run, probe, note, measure = (base[k] for k in ("run", "probe", "note", "measure"))


def amendment():
    request = HERE / "request.json"
    run.require(request.read_bytes() == (QUEUE / f"{OID}.json").read_bytes(), "closure amendment differs from inbox")
    staged = run.read_json(request)
    run.require(staged["id"] == OID and staged["ready"], "wrong closure amendment")
    for name, sha in staged["local_sha256"].items():
        run.require(run.digest(ROOT / name) == sha, f"changed closure amendment input: {name}")
    for path, sha in PUBLIC_PINS.items():
        run.require(run.digest(path) == sha, f"changed public overlay receipt: {path}")
    run.require(run.read_json(PUBLIC_RELEASE)["exit_status"] == 0, "public adapter overlay did not qualify")
    return staged


def candidate_files():
    return {str(CANDIDATE_NEW / name): run.digest((HERE if name == "backend.py" else PRIVATE_REPLAY) / name)
            for name in NAMES}


def public_files():
    files = run.read_json(PUBLIC_QUALIFICATION)["files"]
    run.require(set(files) == {str(PUBLIC_NEW / name) for name in NAMES}, "public overlay file set changed")
    return files


def verify_overlay(folder, manifest):
    with tarfile.open(folder / "raw.tar.gz") as raw:
        get = lambda name: raw.extractfile(f"./{name}").read()
        run.require(get("SETUP-PASS") == b"", "closure overlay proof did not finish")
        pairs = [line.split() for line in get("adapter.sha256").decode().splitlines()]
        run.require({path: sha for sha, path in pairs} == candidate_files(), "installed overlay hashes differ")
        run.require(hashlib.sha256(get("mesh.json")).hexdigest() == run.digest(manifest), "unexpected candidate manifest")
        arms = {}
        for mode in ("product-group", "product-fused"):
            ledger = json.loads(get(f"{mode}/ledger.json"))
            jobs = ledger["jobs"]
            app_xids = {row[name] for row in jobs for name in ("attempt_xid", "effect_xid")}
            settlements = Counter(row["settled_xid"] for row in jobs)
            queue = json.loads(get(f"{mode}/queue-policy.json"))
            writer = json.loads(get(f"{mode}/producer-writer.json"))
            refill = json.loads(get(f"{mode}/refill-ledger.json"))
            fused = mode == "product-fused"
            run.require(len(jobs) == 128 and all(job["state"] == "completed" for job in jobs)
                        and len(app_xids) == 256 and not (app_xids & settlements.keys())
                        and len(ledger["transactions"]) == 1 and queue["queue_policy"] == "success_cohort"
                        and max(settlements.values()) <= 8 and (writer["bulk"], writer["scalar"]) == (1, 0)
                        and refill["fused"] == fused and (refill["counts"] is None) == (not fused),
                        f"{mode} failed native lifecycle")
            arms[mode] = dict(jobs=len(jobs), application_xids=len(app_xids), terminal_xids=len(settlements),
                              completion_refill=refill["counts"])
    return dict(files=candidate_files(), manifest_sha256=run.digest(manifest),
                raw_sha256=run.digest(folder / "raw.tar.gz"), arms=arms)


def overlay_setup(manifest, folder):
    release = QUEUE / f"{OID}.released.json"
    if not release.exists():
        build = run.read_json(manifest)
        pins = (build["files"]["mesh"], run.digest(manifest), run.digest(PRIVATE_REPLAY / "backend.py"),
                run.digest(HERE / "backend.py"))
        folder.mkdir(parents=True)
        probe()
        start = datetime.now(timezone.utc)
        end = start + timedelta(minutes=14)
        note(f"SETUP {OID} {start:%Y-%m-%dT%H:%M:%SZ} {end:%Y-%m-%dT%H:%M:%SZ}\n\n"
             f"Owner: matrix implementation operator. Build-free short16 adapter overlay for {ATTEMPT}.")
        code = 1
        with (run.SUITE / ".timing.lock").open("rb") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            try:
                with (folder / "setup.log").open("xb") as log:
                    code = run.ssh(["bash", "-c", (HERE / "setup.sh").read_text(), "--", *pins],
                                   input=(HERE / "backend.py").read_bytes(), stdout=log, stderr=log,
                                   timeout=600).returncode
            finally:
                with ((folder / "raw.tar.gz").open("xb") as raw, (folder / "export.log").open("xb") as log):
                    export = run.ssh(["tar", "-czf", "-", "-C", str(REMOTE), "."], stdout=raw, stderr=log, timeout=95)
        report = dict(id=OID, exit_status=code or export.returncode, native_exit_status=code,
                      export_exit_status=export.returncode, started=start.isoformat(), receipt=str(folder))
        try:
            probe()
            if report["exit_status"] == 0:
                run.write_json(folder / "qualification.json", verify_overlay(folder, manifest))
        except (OSError, ValueError, KeyError, subprocess.SubprocessError, tarfile.TarError) as err:
            report["qualification_or_release_error"] = str(err)
            run.write_json(folder / "status.json", report)
            raise
        report["released_at"] = run.utc_now()
        run.write_json(folder / "status.json", report)
        run.write_json(release, report)
        note(f"RELEASED {OID} {report['released_at']}, exit {report['exit_status']}. Evidence: {folder}")
    run.require(run.read_json(release)["exit_status"] == 0
                and run.read_json(folder / "qualification.json") == verify_overlay(folder, manifest),
                "closure overlay SETUP did not qualify")


def rebind(document):
    """Move only the four adapter pins/argv entries of each mesh lane to its overlay."""
    candidate, public = candidate_files(), public_files()
    lanes = document["configuration"]["lanes"]
    run.require(sorted(lanes) == sorted(("public-group", "group-a", "group-b", "fused-a", "fused-b",
                                         "river", "oban", "apalis")), "frozen eight lanes changed")
    for label, lane in lanes.items():
        if lane["engine"] != "mesh":
            continue
        old, new, local, files = ((PUBLIC_OLD, PUBLIC_NEW, PUBLIC_REPLAY, public) if label == "public-group"
                                  else (CANDIDATE_OLD, CANDIDATE_NEW, PRIVATE_REPLAY, candidate))
        spec = lane["inventory"]["engines"]["mesh"]
        for name in NAMES:
            expected = spec["verified_files"].pop(str(old / name))
            replacement = files[str(new / name)]
            run.require(expected == run.digest(local / name)
                        and (name == "backend.py" or expected == replacement), "unexpected adapter delta")
            spec["verified_files"][str(new / name)] = replacement
            spec["argv"] = [str(new / name) if arg == str(old / name) else arg for arg in spec["argv"]]
        run.require(spec["argv"][-2] == str(new / "native-run"), f"{label}: launcher not rebound")
        run.check_inventory(lane["inventory"])
    return document


def measured(packet, folder, inventory, groups):
    receipt = QUEUE / f"{OID}-groups-{groups[0]}-{groups[-1]}.released.json"
    if not receipt.exists():
        command = shlex.join([sys.executable, "-B", str(DRIVER), str(QUEUE), str(inventory), str(folder),
                              *map(str, groups)])
        phase = dict(packet, timing_grant_id=OID, timing_window_minutes=28, command_attempts=1, dispatch=command,
                     controller_receipt_dir=STATE / "controller-receipts" / OID)
        run.require(measure(phase, "dispatch", groups) == 0, "fused timing needs reconciliation")
    run.require(run.read_json(receipt)["exit_status"] == 0, "fused timing receipt is unqualified")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    packet = base["intake"]()
    amendment()
    binder = shlex.split(packet["bind_recipe"])[2]
    plan = subprocess.check_output([sys.executable, "-B", binder, "plan"], cwd=run.ROOT)
    run.require(json.loads(plan)["blocks"] == packet["group_plan"], "fused frozen plan changed")
    if args.check:
        subprocess.run(["bash", "-n", str(HERE / "setup.sh")], check=True)
        compile((HERE / "backend.py").read_bytes(), str(HERE / "backend.py"), "exec")
        candidate_files(), public_files()
        print("Fused source, exact plan, closure amendment pins and public overlay receipts verified")
        return
    control = base["public"]["intake"]()
    release = QUEUE / f"{control['id']}.released.json"
    short_release = next(
        path for suffix in ("", "-closure-retry")
        if (path := QUEUE / f"{control['id']}-short-smaller{suffix}-groups-3-5.released.json").is_file()
        and run.read_json(path)["exit_status"] == 0
    )
    run.require(run.read_json(release)["exit_status"] == run.read_json(short_release)["exit_status"] == 0,
                "public short-smaller baseline must release first")
    public_manifest = Path(control["evidence_destination"]) / "setup" / control["attempt"] / "mesh.json"
    folder = Path(packet["evidence_destination"])
    manifest = base["setup"](packet, folder / "setup" / packet["attempt"])
    overlay_setup(manifest, folder / "setup" / f"{packet['attempt']}-closure-retry")
    link = dict(dependency=control["id"], attempt=control["attempt"], manifest_sha256=run.digest(public_manifest),
                receipts={str(path): run.digest(path) for path in (release, public_manifest, short_release)})
    serialized = (json.dumps(link, indent=2, sort_keys=True) + "\n").encode()
    base["write_checked"](QUEUE / packet["public_control_filename"], serialized)
    base["write_checked"](folder / "public-control.json", serialized)
    inventory = folder / "short.inventory.json"
    base["write_checked"](inventory, subprocess.check_output([sys.executable, "-B", binder, "bind",
                          packet["attempt"], manifest, control["attempt"], public_manifest], cwd=run.ROOT))
    rebound = folder / "short.closure-retry.inventory.json"
    base["write_checked"](rebound, (json.dumps(rebind(run.read_json(inventory)), indent=2, sort_keys=True)
                                    + "\n").encode())
    timing = folder / "timing-closure-retry"
    measured(packet, timing, rebound, [1, 2])
    pilot = base["replay"](binder, timing, rebound, 2)
    # Unchanged sealed gate: every paired fused/off row improves both clocks.
    advance = all(row[k] > 0 for rows in pilot["paired"].values() for row in rows
                  for k in ("submit_jobs_per_s", "commit_jobs_per_s"))
    print(f"Fused pilot advancement gate: {advance}", flush=True)
    if advance:
        measured(packet, timing, rebound, [3, 4, 5])
        base["replay"](binder, timing, rebound, 5)


if __name__ == "__main__":
    main()
