"""Qualify a build-free repair of the public product's benchmark adapter."""
from collections import Counter
from datetime import datetime, timezone
import fcntl
import json
from pathlib import Path
import re
import runpy
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[4]
STATE = ROOT / "target/linode-jobs-stepchange"
QUEUE = STATE / "requests"
PRODUCT = HERE.parent.name
ID = f"{PRODUCT}-closure-retry"
ATTEMPT = "trusted-jobs-product-623c46b92cc5"
ELF = "af2aca867a02d526cb6ba92d1867166ffc346f31e97a8f8aba0f19aa76b1d355"
MANIFEST = "979cd601099c5e7da4be763229a91e9dcabca51a35f4bed79f6cf4128adbac55"
FAILED = "47c2482f1710d282b5dd88cd9a1d4ec2a40d6f7c032d612c815d56bdbb92b7b9"
PRODUCT_INPUT = ROOT / "target" / PRODUCT / "source" / HERE.parent.relative_to(ROOT) / "replay"
SOURCE = Path(f"/var/lib/durable-jobs-bench/native/source-{ATTEMPT}/source")
OLD = SOURCE / HERE.parent.relative_to(ROOT) / "replay"
OVERLAY = OLD.with_name("replay-closure-retry")
REMOTE = f"/var/lib/durable-jobs-bench/native/runs/{ATTEMPT}-closure-retry"
OUT = HERE.parent / "evidence/native/setup" / f"{ATTEMPT}-closure-retry"
NAMES = ("native-run", "product_arm.py", "identity.py", "backend.py")
base = runpy.run_path(str(STATE / "trusted-product-setup.py"))
run = base["run"]
from interleave import HOST_IDLE


def files():
    return {str(OVERLAY / name): run.digest((HERE if name == "backend.py" else PRODUCT_INPUT) / name)
            for name in NAMES}


def inputs():
    packet = base["intake"]()
    request = HERE / "request.json"
    staged = run.read_json(request)
    run.require(staged["id"] == ID and request.read_bytes() == (QUEUE / f"{ID}.json").read_bytes(),
                "product adapter repair request differs from inbox")
    for path, digest in staged["local_sha256"].items():
        run.require(run.digest(ROOT / path) == digest, f"product repair input changed: {path}")
    original = Path(packet["evidence_destination"]) / "setup" / ATTEMPT
    proof = run.read_json(original / "qualified.json")
    run.require(packet["attempt"] == ATTEMPT and proof["executable_sha256"] == ELF
                and run.digest(original / "mesh.json") == proof["manifest_sha256"] == MANIFEST
                and run.digest(original / "raw.tar.gz") == proof["raw_sha256"],
                "public product executable or qualified SETUP archive changed")
    path = HERE.parent / "evidence/native/timing/short-smaller/blocks/01-short.1795495340525298000"
    failed = path / "measured-product-bulk/measured-mesh/raw.tar.gz"
    run.require(run.digest(failed) == FAILED, "original product timing failure changed")
    return staged


def grant():
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                        (QUEUE / "STATUS.md").read_text(), flags=re.M)
    mode, id, start, end = events[-1]
    now = datetime.now(timezone.utc)
    run.require((mode, id) == ("SETUP", ID) and datetime.fromisoformat(start) <= now < datetime.fromisoformat(end),
                "product repair requires an active controller SETUP grant")
    return dict(start=start, end=end)


def verify():
    with tarfile.open(OUT / "raw.tar.gz") as raw:
        get = lambda name: raw.extractfile(f"./{name}").read()
        run.require(get("SETUP-PASS") == b"", "benchmark adapter proof did not finish")
        pairs = [line.split() for line in get("adapter.sha256").decode().splitlines()]
        run.require({path: sha for sha, path in pairs} == files(), "installed adapter hashes differ")
        from hashlib import sha256
        run.require(sha256(get("mesh.json")).hexdigest() == MANIFEST, "unexpected product ELF manifest")
        arms = {}
        for mode in ("product-bulk", "product-group"):
            ledger = json.loads(get(f"{mode}/ledger.json"))
            jobs = ledger["jobs"]
            app_xids = {row[name] for row in jobs for name in ("attempt_xid", "effect_xid")}
            settlements = Counter(row["settled_xid"] for row in jobs)
            policy = "set_based" if mode == "product-bulk" else "success_cohort"
            queue = json.loads(get(f"{mode}/queue-policy.json"))
            writer = json.loads(get(f"{mode}/producer-writer.json"))
            run.require(len(jobs) == 128 and all(job["state"] == "completed" for job in jobs)
                        and len(app_xids) == 256 and not (app_xids & settlements.keys())
                        and len(ledger["transactions"]) == 1 and queue["queue_policy"] == policy
                        and max(settlements.values()) <= (1 if mode == "product-bulk" else 8)
                        and (writer["bulk"], writer["scalar"]) == (1, 0), f"{mode} failed native lifecycle")
            arms[mode] = dict(jobs=len(jobs), application_xids=len(app_xids), terminal_xids=len(settlements))
    return dict(files=files(), executable_sha256=ELF, manifest_sha256=MANIFEST,
                raw_sha256=run.digest(OUT / "raw.tar.gz"), arms=arms)


def setup(request):
    with (run.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        active = grant()
        OUT.mkdir(parents=True)
        run.write_json(OUT / "grant.json", active)
        pins = (ELF, MANIFEST, run.digest(PRODUCT_INPUT / "backend.py"), files()[str(OVERLAY / "backend.py")])
        code = None
        try:
            with (OUT / "setup.log").open("xb") as log:
                code = run.ssh(["bash", "-c", (HERE / "setup.sh").read_text(), "--", *pins],
                               input=(HERE / "backend.py").read_bytes(), stdout=log, stderr=log, timeout=600).returncode
        finally:
            run.write_json(OUT / "status.json", dict(exit_code=code, observed=run.utc_now()))
            with (OUT / "raw.tar.gz").open("xb") as raw, (OUT / "export.log").open("xb") as log:
                command = HOST_IDLE + '\nexec tar -czf - -C "$1" .'
                exported = run.ssh(["bash", "-c", command, "--", REMOTE], stdout=raw, stderr=log, timeout=50)
            if code == 0:
                exported.check_returncode()
    if code == 0:
        run.write_json(OUT / "qualification.json", verify())
    return code


def bind(document):
    release = run.read_json(QUEUE / f"{ID}.released.json")
    run.require(release["exit_status"] == 0 and run.read_json(OUT / "qualification.json") == verify(),
                "product repair has no reconciled native receipt")
    for lane in document["configuration"]["lanes"].values():
        if lane["engine"] != "mesh":
            continue
        spec = lane["inventory"]["engines"]["mesh"]
        for name in NAMES:
            original = str(OLD / name)
            replacement = str(OVERLAY / name)
            expected = spec["verified_files"].pop(original)
            new = files()[replacement]
            run.require(expected == run.digest(PRODUCT_INPUT / name)
                        and (name == "backend.py" or expected == new), "unexpected timing adapter delta")
            spec["verified_files"][replacement] = new
            spec["argv"] = [replacement if arg == original else arg for arg in spec["argv"]]
        run.check_inventory(lane["inventory"])
    return document


def timing():
    driver = PRODUCT_INPUT / "controller-timing.py"
    source = driver.read_text()
    old = 'expected = f"{ID}-{plan}"'
    run.require(source.count(old) == 1, "product controller grant binding changed")
    sys.path.insert(0, str(PRODUCT_INPUT))
    exec(compile(source.replace(old, 'expected = f"{ID}-{plan}-closure-retry"'), driver, "exec"),
         dict(__file__=str(driver), __name__="__main__"))


if __name__ == "__main__":
    packet = inputs()
    action = sys.argv.pop(1)
    if action == "check":
        subprocess.run(["bash", "-n", HERE / "setup.sh"], check=True)
        compile((HERE / "backend.py").read_bytes(), str(HERE / "backend.py"), "exec")
        print("Product adapter repair preserves the qualified ELF and all non-adapter pins")
    elif action == "setup":
        sys.exit(setup(packet))
    elif action == "timing":
        timing()
