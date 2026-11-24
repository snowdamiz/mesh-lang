"""Offline binding and guarded entry points for the sole Linode controller."""

from copy import deepcopy
from datetime import datetime, timezone
import fcntl
from importlib import import_module
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile

from audit import BASELINE, PUBLIC_REPLAY, intake
from build import ELF, HERE, MANIFESTS, ROOT, sha

run = intake.run
sys.path.insert(0, str(PUBLIC_REPLAY.parents[1] / "2026-11-23-bulk-enqueue-v2-short/replay"))
matrix = import_module("interleave-v2")
ID = HERE.name
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
BASE = Path("/var/lib/durable-jobs-bench/native")
REMOTE_OUT = BASE / "runs/public-large-file-session"
REMOTE_BROKER = BASE / "source-public-large-file-session/broker"
STOCK = BASE / "builds/trusted-jobs-product-623c46b92cc5/mesh"
MESH = ("product", "file", "inline-a", "inline-b")
LANES = (*MESH, "river", "oban", "apalis")
ORDERS = (
    ("inline-a", "file", "river", "product", "apalis", "inline-b", "oban"),
    ("oban", "inline-b", "apalis", "product", "river", "file", "inline-a"),
    ("product", "oban", "inline-b", "apalis", "file", "river", "inline-a"),
    ("inline-a", "river", "file", "apalis", "inline-b", "oban", "product"),
    ("apalis", "file", "inline-b", "river", "inline-a", "product", "oban"),
)


def document(action):
    doc = intake.read(HERE / "controller-packet.json")
    assert doc["id"] == ID and doc["native_revision"] == intake.read(BASELINE / "large.inventory.json")["artifact"]["version"]
    for name, digest in doc["local_sha256"].items():
        assert sha(ROOT / name) == digest, name
    audit = intake.read(HERE / "evidence/public-baselines.json")
    if action != "check":
        assert doc["status"] == "eligible", "packet is parked"
        assert doc["local_proof"] == "pass"
        for name in ("large", "large-external"):
            assert len(audit["public"][name]["blocks"]) == 5
    assert sha(HERE / "stage.tar.gz") == doc["stage_sha256"]
    return doc


def assignment(mode):
    records = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                         (QUEUE / "STATUS.md").read_text(), re.M)
    kind, key, start, end = records[-1]
    assert (kind, key) == (mode, ID), "requires controller-owned live grant"
    assert datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
    assert (QUEUE / f"{ID}.json").read_bytes() == (HERE / "controller-packet.json").read_bytes()
    return dict(kind=kind, key=key, start=start, end=end)


def bind(broker_sha, doc):
    assert re.fullmatch(r"[a-f0-9]{64}", broker_sha) and broker_sha != ELF
    baseline = intake.read(BASELINE / "large.inventory.json")
    assert baseline["recipe"]["storage"] == "default" and baseline["manifest_sha256"] == MANIFESTS["Linux"]
    baseline["recipe"]["blocks"] = [
        dict(group=i, profile="large", phases=["warmup", "measured"], order=list(order))
        for i, order in enumerate(ORDERS, 1)
    ]
    lanes = baseline["configuration"]["lanes"]
    for label in MESH[1:]:
        lane = deepcopy(lanes["product"])
        spec = lane["inventory"]["engines"]["mesh"]
        argv = spec["argv"]
        pos, = [i for i, item in enumerate(argv) if item.endswith("/replay-closure-retry/native-run")]
        parent = Path(argv[pos]).parent
        replacement = parent.with_name("replay-public-large-file-session")
        argv[pos:pos + 1] = [
            f"PRODUCT_SESSION_MODE={'file' if label == 'file' else 'inline'}",
            f"PRODUCT_SESSION_EXECUTABLE={REMOTE_BROKER}",
            f"PRODUCT_SESSION_SHA256={broker_sha}",
            str(replacement / "native-run"),
        ]
        for name, digest in doc["adapter_sha256"].items():
            assert spec["verified_files"].pop(str(parent / name))
            spec["verified_files"][str(replacement / name)] = digest
        spec["verified_files"][str(REMOTE_BROKER)] = broker_sha
        lanes[label] = lane
    baseline["configuration"]["lanes"] = {name: lanes[name] for name in LANES}
    assert lanes["inline-a"] == lanes["inline-b"]
    return baseline


def validate_seven(config):
    # Only the private seven-lane packet widens the frozen matrix's six-lane
    # validator. Recheck every lane using the unmodified validator in two views.
    assert tuple(config["lanes"]) == LANES and config["profiles"] == ["large"]
    original = validate_seven.original
    original(dict(config, lanes={k: v for k, v in config["lanes"].items() if k != "inline-b"}))
    original(dict(config, lanes={k: v for k, v in config["lanes"].items() if k != "inline-a"}))


def verify_mesh(get, broker, broker_sha):
    ledger = get("ledger")
    assert ledger["input_sha256"] == intake.PROFILES["large"][3]
    jobs = ledger["jobs"]
    expected, = intake.verify.expected_inputs("large").values()
    assert expected == {row["job_key"]: (row["payload_sha256"], row["hold_ms"], row["fail_first"]) for row in jobs}
    assert len(jobs) == 64 and all(row["state"] == "completed" for row in jobs)
    assert len({row[key] for row in jobs for key in ("settled_xid", "attempt_xid", "effect_xid")}) == 192
    assert len(ledger["transactions"]) == 1
    assert get("queue-policy")["queue_policy"] == "set_based"
    storage = get("storage-topology")
    assert (storage["storage"], storage["work_mem"], storage["args_storage"]) == ("default", "4MB", "x")
    counts = get("producer-writer")
    assert (counts["enabled"], counts["bulk"], counts["scalar"]) == (True, 0, 64)
    tls = get("postgres-tls-sessions")
    assert sum(row["role"] == "parity_producer" and row["tls"] for row in tls) == 1
    assert get("build")["files"]["mesh"] == ELF
    ids = get("process-identity")
    assert {tuple(row["cpus"]) for row in ids} == {(0, 1)}
    rows = [row for row in ids if row["argv"][-1] == "run"]
    assert len(rows) == 8 and all(row["argv"][0] == str(STOCK) and row["executable_sha256"] == ELF for row in rows)
    session, = [row for row in ids if row["argv"][-1] == "session"]
    assert session["argv"] == [str(broker), "session"] and session["executable_sha256"] == broker_sha


def setup_result(raw, doc):
    with tarfile.open(raw) as archive:
        def content(name):
            return archive.extractfile(f"./{name}").read()

        assert content("SETUP-PASS") == b""
        digest, path = content("broker.sha256").decode().split()
        assert path == str(REMOTE_BROKER) and re.fullmatch(r"[a-f0-9]{64}", digest) and digest != ELF
        pins = {Path(path).name: digest for digest, path in
                (line.split() for line in content("adapter.sha256").decode().splitlines())}
        assert pins == doc["adapter_sha256"]
        expected_runtime = doc["runtime_sha256"]
        assert expected_runtime == "4e81280d27d218a3c1eaba8b10b33455f06280ca2297b48ad0563ddf96873584"
        assert content("runtime.sha256").decode().split()[0] == expected_runtime
        for mode in ("inline", "file"):
            verify_mesh(lambda key: json.loads(content(f"{mode}/{key}.json")), REMOTE_BROKER, digest)
            assert content(f"{mode}/broker-output.log").splitlines()[-1] == b"{}"
    return dict(broker_sha256=digest, raw_sha256=sha(raw))


def setup(doc, out):
    with (run.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        live = assignment("SETUP")
        out.mkdir()
        run.write_json(out / "grant.json", live)
        code = None
        try:
            with (out / "setup.log").open("xb") as log:
                code = run.ssh(["bash", "-c", (HERE / "native-install.sh").read_text(), "--",
                                doc["stage_sha256"]],
                               input=(HERE / "stage.tar.gz").read_bytes(), stdout=log, stderr=log,
                               timeout=1200).returncode
        finally:
            run.write_json(out / "status.json", dict(exit_code=code, observed=run.utc_now()))
            with (out / "raw.tar.gz").open("xb") as raw, (out / "export.log").open("xb") as log:
                command = matrix.HOST_IDLE + '\nexec tar -czf - -C "$1" .'
                exported = run.ssh(["bash", "-c", command, "--", str(REMOTE_OUT)],
                                   stdout=raw, stderr=log, timeout=50)
            if code == 0:
                exported.check_returncode()
    if code == 0:
        run.write_json(out / "qualified.json", setup_result(out / "raw.tar.gz", doc))
    return code


def replay(qualification, path, block):
    scores = intake.compare(qualification, path, block["group"])
    broker = qualification["broker_sha256"]
    for phase in block["phases"]:
        for label in MESH:
            folder = path / f"{phase}-{label}/{phase}-mesh"
            with tarfile.open(folder / "raw.tar.gz") as archive:
                verify_mesh(lambda key: run.file_json(archive, f"adapter/{key}.json"),
                            STOCK if label == "product" else REMOTE_BROKER,
                            ELF if label == "product" else broker)
                assert archive.extractfile("adapter/broker-output.log").read().splitlines()[-1] == b"{}"
    a, b, direct, stock = (scores[label]["first_submit_to_commit_ms"]
                           for label in ("inline-a", "inline-b", "file", "product"))
    return dict(scores=scores, producer=dict(faster_duplicate_minus_file_ms=min(a, b) - direct,
                                             duplicate_spread_ms=abs(a - b), original_minus_file_ms=stock - direct))


def timing(doc, setup_dir, out):
    assignment("TIMING")
    release = intake.read(QUEUE / f"{ID}.released.json")
    assert release["exit_status"] == 0
    proof = setup_result(setup_dir / "raw.tar.gz", doc)
    assert intake.read(setup_dir / "qualified.json") == proof
    manifest = bind(proof["broker_sha256"], doc)
    manifest["broker_sha256"] = proof["broker_sha256"]
    matrix.ATTEMPT_ERRORS += (AssertionError,)
    matrix.plan = lambda *_: iter(manifest["recipe"]["blocks"])
    matrix.replay = lambda path, _, block: replay(manifest, path, block)
    validate_seven.original = matrix.validate
    matrix.validate = validate_seven
    original = matrix.dispatch_block

    def dispatch_block(*args):
        assignment("TIMING")
        return original(*args)

    matrix.dispatch_block = dispatch_block
    return matrix.dispatch(out, manifest["configuration"], [1, 2, 3, 4, 5])


def check(doc):
    synthetic_sha = "1" * 64
    comparison = bind(synthetic_sha, doc)
    validate_seven.original = matrix.validate
    validate_seven(comparison["configuration"])
    assert all(set(order) == set(LANES) for order in ORDERS)
    assert ORDERS[0] == ORDERS[1][::-1] and ORDERS[2] == ORDERS[3][::-1]
    subprocess.run(["bash", "-n", HERE / "native-install.sh"], check=True)
    return comparison


if __name__ == "__main__":
    action, *values = sys.argv[1:]
    packet = document(action)
    if action == "check":
        print(json.dumps(check(packet), indent=2))
    elif action == "setup":
        sys.exit(setup(packet, Path(values[0]).resolve()))
    elif action == "timing":
        sys.exit(timing(packet, *(Path(path).resolve() for path in values)))
    else:
        sys.exit("expected check, setup or timing")
