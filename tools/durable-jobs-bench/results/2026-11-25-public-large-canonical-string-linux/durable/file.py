"""Controller-only dispatch and offline raw replay for the earned public FILE trial."""
from copy import deepcopy
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import sys
import tempfile

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent
RESULTS = PARENT.parent
sys.path.insert(0, str(PARENT))
from replay import digest, extract, qualification, screen

EXT = RESULTS / "2026-11-25-public-large-file-external"
sys.path.insert(0, str(EXT))
import matched

run = matched.run
matrix = matched.matrix
read = matched.read
PACKET = HERE / "controller-packet.json"
LANES = ("control-a", "candidate", "exact-file", "control-b")


def file_configuration(proof):
    published = matched.configure()
    source = read(EXT / "grant-parser-retry/evidence/native/series.json")
    assert published["configuration"] == source["config"]
    old_file = source["config"]["lanes"]["file"]
    broker = str(matched.parked.REMOTE_BROKER)
    assert proof["bindings"]["exact-file"]["path"] == broker
    assert proof["bindings"]["exact-file"]["sha256"] == matched.BROKER
    labels = {"control-a":"stock","control-b":"stock","candidate":"candidate","exact-file":"exact-file"}
    lanes = {}
    for name in LANES:
        entry = deepcopy(old_file)
        inventory = entry["inventory"]["engines"]["mesh"]
        pinned = proof["bindings"][labels[name]]
        expected = [f"PRODUCT_SESSION_EXECUTABLE={pinned['path']}" if value.startswith("PRODUCT_SESSION_EXECUTABLE=")
                    else f"PRODUCT_SESSION_SHA256={pinned['sha256']}" if value.startswith("PRODUCT_SESSION_SHA256=")
                    else value for value in inventory["argv"]]
        assert expected == pinned["argv"]
        inventory["argv"] = expected
        if name != "exact-file":
            del inventory["verified_files"][broker]
            inventory["verified_files"][pinned["path"]] = pinned["sha256"]
        lanes[name] = entry
    assert lanes["exact-file"] == old_file and lanes["control-a"] == lanes["control-b"]
    orders = [LANES, LANES[::-1],
              ("candidate", "control-b", "control-a", "exact-file"),
              ("exact-file", "control-a", "control-b", "candidate"),
              ("control-b", "exact-file", "candidate", "control-a")]
    blocks = [dict(group=i,profile="large",phases=["warmup","measured"],order=list(order))
              for i, order in enumerate(orders,1)]
    cfg = dict(source["config"], lanes=lanes,
               description="Public LARGE FILE producer relink against two identical stock controls and original FILE ELF")
    return dict(published, configuration=cfg, recipe=dict(published["recipe"], blocks=blocks))


def bind_file(qualified, screened, linked):
    with tempfile.TemporaryDirectory(dir=HERE) as scratch:
        temp = Path(scratch)
        bind = qualification(qualified, temp / "qualification")
        bound_screen = screen(screened, temp / "screen", bind)
        assert bound_screen["earned"] is True
        release = read(linked / "controller-release.json")
        packet = read(PACKET)
        assert (release["id"], release["exit_status"], release["request_sha256"]) == (
            packet["phases"]["file_qualification"]["id"], 0, digest(PACKET))
        assert (release["screen_sha256"], release["suffix"]) == (bound_screen["screen_sha256"], bound_screen["suffix"])
        raw = linked / "raw.tar.gz"
        assert release["raw_sha256"] == digest(raw)
        extract(raw, temp / "file")
        result = temp / "file/qualified.json"
        assert digest(result) == release["qualified_sha256"]
        proof = read(result)
        assert proof["screen_sha256"] == bound_screen["screen_sha256"]
        base = f"/var/lib/durable-jobs-bench/native/source-public-large-canonical-string-linux-{release['suffix']}-file/bin"
        assert all(proof["bindings"][lane]["path"] == f"{base}/{lane}" for lane in ("stock","candidate"))
        assert proof["bindings"]["stock"]["sha256"] != proof["bindings"]["candidate"]["sha256"]
        assert re.fullmatch(r"[a-f0-9]{64}", proof["bindings"]["candidate"]["sha256"])
        assert re.fullmatch(r"[a-f0-9]{64}", proof["bindings"]["stock"]["sha256"])
        for lane in ("stock", "candidate"):
            assert proof["bindings"][lane]["sha256"] in (temp / "file/brokers.sha256").read_text()
        return proof, file_configuration(proof)


def check_grant():
    doc = read(PACKET)
    phase = doc["phases"]["file_timing"]
    request = (matched.QUEUE / f"{phase['id']}.json")
    assert request.is_file() and request.read_bytes() == PACKET.read_bytes()
    state = (matched.QUEUE / "STATUS.md").read_text()
    line = re.findall(r"^(?:SETUP|TIMING|RELEASED|HOLD)\b[^\n]*", state, re.M)[-1]
    mode, key, start, end = line.split()
    assert (mode, key) == ("TIMING", phase["id"])
    assert datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
    return dict(id=key,start=start,end=end,request_sha256=digest(PACKET))


def score_block(proof, plan, folder, block):
    # Reuse the original scorer and physical-work checks. Only expected producer
    # identity varies with the lane. Every worker and migrator stays original.
    scores = matched.original.intake.compare(plan, folder, block["group"])
    receipts = {}
    cfg = plan["configuration"]
    for phase, lane, arm in matrix.arms(folder, cfg, block):
        entry = cfg["lanes"][lane]
        score = scores[lane] if phase == "measured" else run.score_arm(
            arm, block, phase, "mesh", entry["inventory"])
        label = "stock" if lane.startswith("control-") else lane
        broker = proof["bindings"][label]
        old_path, old_sha = matched.parked.REMOTE_BROKER, matched.BROKER
        matched.parked.REMOTE_BROKER = Path(broker["path"])
        matched.BROKER = broker["sha256"]
        try:
            receipts.setdefault(phase, {})[lane] = matched.arm_receipt(arm, plan, lane, score)
        finally:
            matched.parked.REMOTE_BROKER, matched.BROKER = old_path, old_sha
    clocks = {lane:{"enqueue_ms":row["first_submit_to_commit_ms"],
                    "submit_ms":(row["last_settlement_us"] - row["first_submit_us"]) / 1000,
                    "drain_ms":(row["last_settlement_us"] - row["first_commit_us"]) / 1000}
              for lane,row in scores.items()}
    return dict(scores=scores, clocks=clocks, receipts=receipts)


def timing(proof, plan, out):
    grant = check_grant()
    out.mkdir(parents=True)
    run.write_json(out / "grant.json", grant)
    matrix.plan = lambda *_: iter(plan["recipe"]["blocks"])
    matrix.replay = lambda path, _, block: score_block(proof, plan, path, block)
    dispatch = matrix.dispatch_block
    arm = run.dispatch_arm

    def block_dispatch(*args):
        check_grant()
        assert dispatch(*args), "block failed reconciliation, preserve all raw attempts"
        return True

    def arm_dispatch(*args):
        check_grant()
        return arm(*args)

    matrix.dispatch_block = block_dispatch
    run.dispatch_arm = arm_dispatch
    return matrix.dispatch(out, plan["configuration"], [])


def report(proof, plan, out):
    grant = read(out / "grant.json")
    release = read(out / "controller-release.json")
    assert release["grant"] == grant
    assert (release["id"],release["exit_status"],grant["request_sha256"]) == (
        read(PACKET)["phases"]["file_timing"]["id"], 0, digest(PACKET))
    series = read(out / "series.json")
    assert series["config"] == plan["configuration"] and series["plan"] == plan["recipe"]["blocks"]
    groups = []
    for block in series["plan"]:
        path, = [p for p in matrix.attempt_paths(out, block) if (p / "reconciled.json").is_file()]
        row = score_block(proof, plan, path, block)
        assert row == read(path / "reconciled.json")
        clocks = row["clocks"]
        row["comparisons"] = {key:{
            "faster_duplicate_minus_candidate_ms":min(clocks[a][key] for a in ("control-a","control-b")) - clocks["candidate"][key],
            "exact_original_minus_candidate_ms":clocks["exact-file"][key] - clocks["candidate"][key],
            "duplicate_spread_ms":abs(clocks["control-a"][key] - clocks["control-b"][key]),
        } for key in matched.CLOCKS}
        groups.append(row)
    comparisons = {key:{field:matched.distribution([row["comparisons"][key][field] for row in groups])
                        for field in groups[0]["comparisons"][key]} for key in matched.CLOCKS}
    return dict(groups=groups, comparisons=comparisons)


if __name__ == "__main__":
    mode, qual, comp, link, output = sys.argv[1:]
    proof, plan = bind_file(Path(qual),Path(comp),Path(link))
    if mode == "timing":
        sys.exit(timing(proof, plan, Path(output)))
    assert mode == "replay"
    print(json.dumps(report(proof, plan, Path(output)), sort_keys=True, indent=2, allow_nan=False))
