"""Sealed continuation of the parked public LARGE file-session request."""

from copy import deepcopy
import json
from pathlib import Path
from statistics import median
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
# Import by filename because this entry point and the parked controller share a name.
from importlib.util import module_from_spec, spec_from_file_location

spec = spec_from_file_location("parked_controller", HERE.parent / "controller.py")
parked = module_from_spec(spec)
spec.loader.exec_module(parked)
from build import ROOT, sha

ID = HERE.parent.name + "-resume"
PACKET = HERE / "intake.json"
STAGE = HERE / "stage.tar.gz"
EXACT = ("recovered-exact", "rebuilt-exact")
RELINK = "same-source-same-compiler-relink"
parked.ID = ID
parked.REMOTE_OUT = parked.BASE / "runs/public-large-file-session-resume"
parked.REMOTE_BROKER = parked.BASE / "source-public-large-file-session-resume/broker"
RELINK_BROKER = parked.REMOTE_BROKER.with_name("relink-control")
run, matrix, intake = parked.run, parked.matrix, parked.intake
FROZEN_VALIDATE = matrix.validate


def document():
    doc = intake.read(PACKET)
    assert (doc["id"], doc["ready"]) == (ID, True)
    assert sha(STAGE) == doc["operational_stage_sha256"]
    for name, digest in doc["local_sha256"].items():
        assert sha(ROOT / name) == digest, name
    audit = intake.read(HERE / "evidence/public-baselines.json")
    assert all({b["group"] for b in audit["public"][profile]["blocks"]} == set(range(1, 6))
               for profile in ("large", "large-external"))
    trace = intake.read(HERE / "evidence/trace-inline.json")
    original = intake.read(HERE.parent / "evidence/trace-file-replay.json")
    assert len(trace["frames"]) == 64
    assert [dict(bytes=row["bytes"], jobs=row["count"], sha256=row["sha256"])
            for row in trace["frames"]] == original["frames"]
    assert trace["fixture_sha256"] == parked.intake.PROFILES["large"][3]
    return doc


def assignment(mode):
    import re
    from datetime import datetime, timezone

    line = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                      (parked.QUEUE / "STATUS.md").read_text(), re.M)[-1]
    kind, key, start, end = line
    assert (kind, key) == (mode, ID), "requires controller-owned live grant"
    assert datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
    assert (parked.QUEUE / f"{ID}.json").read_bytes() == PACKET.read_bytes()
    return dict(kind=kind, key=key, start=start, end=end)


def sha_member(archive, name, path):
    digest, resolved = archive.extractfile(f"./{name}.sha256").read().decode().split()
    assert resolved == str(path) and len(bytes.fromhex(digest)) == 32
    return digest


def qualification(raw, doc):
    with tarfile.open(raw) as archive:
        def get(name):
            return archive.extractfile(f"./{name}").read()

        assert get("SETUP-PASS") == b""
        broker = sha_member(archive, "broker", parked.REMOTE_BROKER)
        runtime = sha_member(archive, "runtime", parked.REMOTE_BROKER.with_name("libmesh_rt.a"))
        origin = get("runtime-origin.txt").decode().strip()
        relink = None
        if runtime == doc["runtime_sha256"]:
            assert origin in EXACT
        else:
            assert origin == RELINK
            relink = sha_member(archive, "relink-control", RELINK_BROKER)
            assert relink not in (broker, parked.ELF)
        assert broker != parked.ELF
        pins = {Path(path).name: digest for digest, path in
                (line.split() for line in get("adapter.sha256").decode().splitlines())}
        assert pins == doc["adapter_sha256"]
        checks = {"inline": (parked.REMOTE_BROKER, broker), "file": (parked.REMOTE_BROKER, broker)}
        if relink:
            checks["relink-inline"] = (RELINK_BROKER, relink)
        for label, (exe, digest) in checks.items():
            parked.verify_mesh(lambda key: json.loads(get(f"{label}/{key}.json")), exe, digest)
            assert get(f"{label}/broker-output.log").splitlines()[-1] == b"{}"
        proofs = {}
        for label in ("file", "inline", "stock"):
            proof = json.loads(get(f"proof-owner/transactions/{label}/proof.json"))
            assert proof["scalar_calls"] == 64 and len(proof["persisted"]) == 64
            assert proof["backend"]["tls"] and proof["backend"]["producer_sessions"] == 1
            assert proof["fixture_sha256"] == doc["input_sha256"]
            assert proof["input_sha256"] == doc["fixture_file_sha256"]
            proofs[label] = proof
        direct = proofs["file"]
        assert direct["late_rollback_prior_writes"] == 62
        assert direct["explicit_rollback"]["count"] == 1
        assert direct["lost_reply"]["count"] == direct["retry"]["count"] == 64
        assert direct["lost_reply"]["ids"] == direct["retry"]["ids"]
        assert direct["lost_reply"]["xid"] != direct["retry"]["xid"]
    return dict(broker_sha256=broker, runtime_sha256=runtime, runtime_origin=origin,
                relink_sha256=relink, raw_sha256=sha(raw))


def bind(proof, doc):
    plan = parked.bind(proof["broker_sha256"], doc)
    relink = proof["relink_sha256"]
    if relink:
        lanes = plan["configuration"]["lanes"]
        lane = deepcopy(lanes["inline-a"])
        engine = lane["inventory"]["engines"]["mesh"]
        argv = engine["argv"]
        argv[argv.index(f"PRODUCT_SESSION_EXECUTABLE={parked.REMOTE_BROKER}")] = (
            f"PRODUCT_SESSION_EXECUTABLE={RELINK_BROKER}")
        argv[argv.index(f"PRODUCT_SESSION_SHA256={proof['broker_sha256']}")] = f"PRODUCT_SESSION_SHA256={relink}"
        del engine["verified_files"][str(parked.REMOTE_BROKER)]
        engine["verified_files"][str(RELINK_BROKER)] = relink
        lanes["relink-inline"] = lane
        for block, position in zip(plan["recipe"]["blocks"], (2, 5, 4, 3, 6), strict=True):
            block["order"].insert(position, "relink-inline")
    plan["session_qualification"] = proof
    return plan


def validate(config):
    lanes = config["lanes"]
    assert tuple(lanes) in (parked.LANES, (*parked.LANES, "relink-inline"))
    assert lanes["inline-a"] == lanes["inline-b"]
    for removed in ("inline-a", "inline-b"):
        FROZEN_VALIDATE(dict(config, lanes={key: lane for key, lane in lanes.items()
                                           if key not in (removed, "relink-inline")}))
    if "relink-inline" in lanes:
        FROZEN_VALIDATE(dict(config, lanes={key: lanes[key]
                                           for key in ("product", "relink-inline", "river", "oban", "apalis")}))


def replay(plan, path, block):
    scores = intake.compare(plan, path, block["group"])
    proof = plan["session_qualification"]
    brokers = {label: (parked.REMOTE_BROKER, proof["broker_sha256"]) for label in parked.MESH[1:]}
    brokers["product"] = (parked.STOCK, parked.ELF)
    if proof["relink_sha256"]:
        brokers["relink-inline"] = (RELINK_BROKER, proof["relink_sha256"])
    for phase in block["phases"]:
        for label, (exe, digest) in brokers.items():
            folder = path / f"{phase}-{label}/{phase}-mesh"
            with tarfile.open(folder / "raw.tar.gz") as archive:
                parked.verify_mesh(lambda key: run.file_json(archive, f"adapter/{key}.json"), exe, digest)
                assert archive.extractfile("adapter/broker-output.log").read().splitlines()[-1] == b"{}"
    clocks = {
        label: dict(enqueue_ms=score["first_submit_to_commit_ms"],
                    submit_ms=(score["last_settlement_us"] - score["first_submit_us"]) / 1000,
                    drain_ms=(score["last_settlement_us"] - score["first_commit_us"]) / 1000,
                    jobs_s=score["submit_jobs_per_s"], drain_jobs_s=score["commit_jobs_per_s"])
        for label, score in scores.items()
    }
    return dict(scores=scores, clocks=clocks,
                faster_duplicate_minus_file_ms={
                    key: min(clocks[a][key] for a in ("inline-a", "inline-b")) - clocks["file"][key]
                    for key in ("enqueue_ms", "submit_ms", "drain_ms")},
                duplicate_spread_ms={
                    key: abs(clocks["inline-a"][key] - clocks["inline-b"][key])
                    for key in ("enqueue_ms", "submit_ms", "drain_ms")},
                paired_ms={
                    label: {key: clocks[label][key] - clocks["file"][key]
                            for key in ("enqueue_ms", "submit_ms", "drain_ms")}
                    for label in clocks if label != "file"})


def check(doc):
    subprocess.run(["bash", "-n", HERE / "native-install.sh"], check=True)
    for relink in (None, "2" * 64):
        plan = bind(dict(broker_sha256="1" * 64, relink_sha256=relink), doc)
        cfg = plan["configuration"]
        validate(cfg)
        original = intake.read(parked.BASELINE / "large.inventory.json")["configuration"]["lanes"]
        assert all(cfg["lanes"][lane] == original[lane] for lane in original)
        for block, order in zip(plan["recipe"]["blocks"], parked.ORDERS, strict=True):
            assert tuple(k for k in block["order"] if k != "relink-inline") == order
            assert len(block["order"]) == len(cfg["lanes"]) == len(set(block["order"]))
    print("10 audited blocks, 64 inline/file frame matches, and exact and relink timing plans verified")


def setup(doc, out):
    # The parked method already uses the campaign lock and archives failures.
    parked.HERE = HERE
    parked.assignment = assignment
    parked.setup_result = qualification
    return parked.setup(dict(doc, stage_sha256=doc["operational_stage_sha256"]), out)


def timing_plan(doc, setup_dir):
    proof = qualification(setup_dir / "raw.tar.gz", doc)
    assert intake.read(setup_dir / "qualified.json") == proof
    return bind(proof, doc)


def timing(doc, setup_dir, out):
    assignment("TIMING")
    release = intake.read(parked.QUEUE / f"{ID}.released.json")
    assert (release["id"], release["phase"], release["exit_status"], release["receipt"]) == (
        ID, "setup", 0, str(setup_dir))
    plan = timing_plan(doc, setup_dir)
    validate(plan["configuration"])
    matrix.ATTEMPT_ERRORS += (AssertionError,)
    matrix.plan = lambda *_: iter(plan["recipe"]["blocks"])
    matrix.replay = lambda path, _, block: replay(plan, path, block)
    matrix.validate = validate
    dispatch_block = matrix.dispatch_block

    def dispatch(*args):
        assignment("TIMING")
        return dispatch_block(*args)

    matrix.dispatch_block = dispatch
    return matrix.dispatch(out, plan["configuration"], [1, 2, 3, 4, 5])


def scores(doc, setup_dir, out):
    plan = timing_plan(doc, setup_dir)
    series = intake.read(out / "series.json")
    assert series["config"] == plan["configuration"] and series["plan"] == plan["recipe"]["blocks"]
    rows = []
    for block in series["plan"]:
        candidates = [p for p in matrix.attempt_paths(out, block) if (p / "reconciled.json").is_file()]
        assert len(candidates) == 1, (block["group"], candidates)
        path, = candidates
        row = replay(plan, path, block)
        assert row == intake.read(path / "reconciled.json")
        rows.append(dict(group=block["group"], path=path.name, **row))
    def distribution(values):
        return dict(n=len(values), median=median(values), min=min(values), max=max(values))

    summary = {label: {key: distribution([row["clocks"][label][key] for row in rows])
                       for key in rows[0]["clocks"][label]}
               for label in rows[0]["clocks"]}
    return dict(qualification=plan["session_qualification"], summary=summary, groups=rows)


if __name__ == "__main__":
    action, *args = sys.argv[1:]
    packet = document()
    if action == "check":
        check(packet)
    elif action == "setup":
        sys.exit(setup(packet, Path(args[0]).resolve()))
    elif action in ("timing", "scores"):
        result = globals()[action](packet, *(Path(value).resolve() for value in args))
        if action == "scores":
            print(json.dumps(result, sort_keys=True, indent=2))
        else:
            sys.exit(result)
    else:
        sys.exit("expected check, setup, timing or scores")
