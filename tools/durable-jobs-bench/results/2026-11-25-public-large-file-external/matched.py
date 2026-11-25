"""Bind the two released inventories and replay the matched EXTERNAL series."""

from copy import deepcopy
from datetime import datetime, timezone
from pathlib import Path
import json
import re
import runpy
from statistics import median
import sys
import tarfile

HERE = Path(__file__).resolve().parent
RESULTS = HERE.parent
SOURCE = RESULTS / "2026-11-24-trusted-jobs-product-composition"
SESSION = RESULTS / "2026-11-24-public-large-file-session" / "resume"
DEFAULT = SESSION / "evidence/native/timing-adapter-path-retry"
RECOVERED = SESSION / "evidence/native/setup-export-recovered"
EXTERNAL = SOURCE / "evidence/native/timing-closure-retry"
PACKET = HERE / "controller-packet.json"
OUT = HERE / "evidence/native"
ID = HERE.name
BROKER = "a5a2d5667c2d7209234d7f1468c40dd4071818010be31364a4e82d04b7ff18a3"
PEERS = ("river", "oban", "apalis")
EXPERIMENT = ("file", "inline-a", "inline-b")
CLOCKS = ("enqueue_ms", "submit_ms", "drain_ms")

# Use the checked-in public scorer, including on a fresh checkout.
sys.path[:0] = [str(SESSION), str(SOURCE / "replay"),
                str(RESULTS / "2026-11-23-bulk-enqueue-v2-short/replay")]
import intake

prior = runpy.run_path(str(SESSION / "adapter-path-retry/retry.py"))
original = prior["original"]
parked, run, matrix = original.parked, original.run, original.matrix
read, sha = run.read_json, run.digest
QUEUE = parked.QUEUE


def packet():
    doc = read(PACKET)
    assert doc["id"] == ID and doc["ready"] is True
    for path, digest in doc["local_sha256"].items():
        assert sha(parked.ROOT / path) == digest, path
    return doc


def session_document():
    """Verify the retained seals using tracked copies of the old target inputs."""
    doc = read(original.PACKET)
    assert (doc["id"], doc["ready"]) == (original.ID, True)
    assert sha(original.STAGE) == doc["operational_stage_sha256"]
    prefix = f"target/{SOURCE.name}/source/"
    for path, digest in doc["local_sha256"].items():
        assert sha(parked.ROOT / path.removeprefix(prefix)) == digest, path
    return doc


def configure():
    public = read(EXTERNAL / "large-external.inventory.json")
    default = read(DEFAULT / "series.json")
    release = read(DEFAULT.with_name(DEFAULT.name + "-release.json"))
    assert (release["id"], release["exit_status"]) == (prior["ID"], 0)
    doc = session_document()
    old = prior["plan"](doc, RECOVERED)
    assert (old["configuration"], old["recipe"]["blocks"]) == (default["config"], default["plan"])
    proof = old["session_qualification"]
    setup = read(RECOVERED / "controller-release.json")
    assert (setup["exit_status"], setup["broker_sha256"], setup["recovered_raw_sha256"]) == (
        0, BROKER, proof["raw_sha256"])
    assert proof["broker_sha256"] == BROKER and proof["relink_sha256"] is None
    assert proof["runtime_sha256"] == doc["runtime_sha256"] == setup["runtime_sha256"]
    member_pins = read(RECOVERED / "member-pins.json")
    assert member_pins["exit_status"] == 0 and member_pins["raw_sha256"] == proof["raw_sha256"]
    assert member_pins["log_sha256"] == sha(RECOVERED / "member-pins.txt")
    assert setup["member_pins_sha256"] == sha(RECOVERED / "member-pins.json")

    assert public["artifact"] == old["artifact"] and public["artifact"]["files"]["mesh"] == parked.ELF
    assert public["manifest_sha256"] == old["manifest_sha256"]
    for key in ("host", "cpus", "clock_ticks", "contract", "profiles"):
        assert public["configuration"][key] == old["configuration"][key], key
    assert public["configuration"]["cpus"] == [0, 1]
    recipe = public["recipe"]
    assert recipe == dict(old["recipe"], storage="external-4m", blocks=recipe["blocks"])
    assert (recipe["product_runners"], recipe["peer_runners"], recipe["slots"], recipe["cap"]) == (8, 8, 64, 96)
    assert recipe["input_sha256"] == doc["input_sha256"]
    historical = read(EXTERNAL / "large-external/series.json")
    assert historical["config"] == public["configuration"] and historical["plan"] == recipe["blocks"]
    historical_groups = []
    for path in sorted((EXTERNAL / "large-external/controller-evidence").glob("*.released.json")):
        released = read(path)
        assert released["id"] == SOURCE.name + "-large-external-closure-retry"
        assert released["exit_status"] == 0
        historical_groups.extend(released["groups"])
    assert sorted(historical_groups) == list(range(1, 6))
    drivers = {Path(path).name: sha(Path(path)) for path in
               (matrix.__file__, run.__file__, run.SUITE / "verify.py", run.SUITE / "worker.py")}
    assert drivers == default["drivers"] == historical["drivers"]
    assert original.intake is intake

    lanes = public["configuration"]["lanes"]
    previous = old["configuration"]["lanes"]
    stock_default = deepcopy(lanes["product"])
    argv = stock_default["inventory"]["engines"]["mesh"]["argv"]
    argv[argv.index("PRODUCT_STORAGE=external-4m")] = "PRODUCT_STORAGE=default"
    assert stock_default == previous["product"]
    for name in EXPERIMENT:
        lane = deepcopy(previous[name])
        spec = lane["inventory"]["engines"]["mesh"]
        argv = spec["argv"]
        argv[argv.index("PRODUCT_STORAGE=default")] = "PRODUCT_STORAGE=external-4m"
        assert f"PRODUCT_SESSION_MODE={'file' if name == 'file' else 'inline'}" in argv
        assert spec["verified_files"][str(parked.REMOTE_BROKER)] == BROKER
        lanes[name] = lane
    assert lanes["inline-a"] == lanes["inline-b"]
    config = public["configuration"]
    config["description"] = "Public LARGE on symmetric EXTERNAL storage with same-broker FILE and inline controls."
    config["lanes"] = {label: lanes[label] for label in parked.LANES}
    recipe["blocks"] = old["recipe"]["blocks"]
    original.validate(config)
    return dict(public, session_qualification=proof)


def distribution(values):
    return dict(n=len(values), median=median(values), min=min(values), max=max(values))


def arm_receipt(folder, plan, lane, score):
    cfg = plan["configuration"]
    engine = cfg["lanes"][lane]["engine"]
    raw = folder / "raw.tar.gz"
    with tarfile.open(raw) as archive:
        def get(name):
            return run.file_json(archive, f"adapter/{name}.json")

        ledger = get("ledger")
        rows = ledger["jobs"]
        assert len(rows) == 64 and len(ledger["transactions"]) == 1
        assert all(row["state"] == "completed" and row["attempt"] == row["attempts"] == row["effects"] == 1
                   for row in rows)
        app_xids = {row[k] for row in rows for k in ("attempt_xid", "effect_xid")}
        terminal = {row["settled_xid"] for row in rows}
        assert len(app_xids) == 128 and not (app_xids & terminal)
        hashes = {row["job_key"]: row["payload_sha256"] for row in rows}

        if engine == "mesh":
            settings = get("storage-topology")
            assert settings == dict(args_compression="", args_storage="e", default_toast_compression="pglz",
                                    max_connections=96, shared_buffers="128MB", storage="external-4m",
                                    topology="stock", wal_compression="off", work_mem="4MB")
            assert len(terminal) == 64 and get("database")["durable"] is True
            producer = get("producer-writer")
            assert (producer["enabled"], producer["bulk"], producer["scalar"]) == (True, 0, 64)
            assert sum(r["role"] == "parity_producer" and r["tls"] for r in get("postgres-tls-sessions")) == 1
            identities = get("process-identity")
            assert len(identities) == 9 and {tuple(row["cpus"]) for row in identities} == {(0, 1)}
            broker = parked.STOCK if lane == "product" else parked.REMOTE_BROKER
            broker_sha = parked.ELF if lane == "product" else BROKER
            assert sum(r["argv"] == [str(parked.STOCK), "run"] and
                       r["executable_sha256"] == parked.ELF for r in identities) == 8
            assert sum(r["argv"] == [str(broker), "session"] and
                       r["executable_sha256"] == broker_sha for r in identities) == 1
            assert archive.extractfile("adapter/broker-output.log").read().splitlines()[-1] == b"{}"
            storage = settings
        else:
            storage = get("payload-storage")
            installed = storage["setup"]
            persisted = storage["persisted"]
            assert storage["policy"] == "EXTERNAL" and installed["rows_before"] == 0
            assert installed["before"]["attstorage"] == "x"
            assert persisted["metadata"] == installed["after"] and installed["after"]["attstorage"] == "e"
            assert installed["ddl_start_us"] <= installed["ddl_end_us"] <= score["first_submit_us"]
            assert score["last_settlement_us"] <= storage["verify_start_us"] <= storage["verify_end_us"]
            assert len(persisted["rows"]) == 64 and all(
                row["compression"] is None and row["payload_bytes"] == 524288 for row in persisted["rows"])
            assert hashes == {row["job_key"]: row["payload_sha256"] for row in persisted["rows"]}
            if engine == "go":
                assert storage["triggers_retired"] is True
            storage = dict(policy="EXTERNAL", relation=persisted["metadata"]["relation"],
                           toast_heap_bytes=persisted["toast_heap_bytes"],
                           ddl_end_us=installed["ddl_end_us"], verify_start_us=storage["verify_start_us"])
    return dict(raw_sha256=sha(raw), storage=storage, app_xids=len(app_xids), terminal_xids=len(terminal))


def replay_block(plan, path, block, *, matched=True):
    scores = original.intake.compare(plan, path, block["group"])
    receipts = {}
    for phase, lane, folder in matrix.arms(path, plan["configuration"], block):
        entry = plan["configuration"]["lanes"][lane]
        phase_score = scores[lane] if phase == "measured" else run.score_arm(
            folder, block, phase, entry["engine"], entry["inventory"])
        receipts.setdefault(phase, {})[lane] = arm_receipt(folder, plan, lane, phase_score)
    clocks = {lane: dict(enqueue_ms=row["first_submit_to_commit_ms"],
                         submit_ms=(row["last_settlement_us"] - row["first_submit_us"]) / 1000,
                         drain_ms=(row["last_settlement_us"] - row["first_commit_us"]) / 1000,
                         jobs_s=row["submit_jobs_per_s"], drain_jobs_s=row["commit_jobs_per_s"])
              for lane, row in scores.items()}
    result = dict(scores=scores, clocks=clocks, receipts=receipts)
    if not matched:
        return result
    return dict(result,
                faster_inline_minus_file_ms={key: round(
                    min(clocks[a][key] for a in EXPERIMENT[1:]) - clocks["file"][key], 3) for key in CLOCKS},
                inline_spread_ms={key: round(abs(clocks["inline-a"][key] - clocks["inline-b"][key]), 3)
                                  for key in CLOCKS},
                lane_minus_file_ms={lane: {key: round(row[key] - clocks["file"][key], 3) for key in CLOCKS}
                                    for lane, row in clocks.items() if lane != "file"})


def report(plan):
    release = read(OUT / "controller-release.json")
    assert (release["id"], release["exit_status"], release["grant"]["request_sha256"]) == (ID, 0, sha(PACKET))
    assert release["grant"] == read(OUT / "grant.json")
    series = read(OUT / "series.json")
    assert series["config"] == plan["configuration"] and series["plan"] == plan["recipe"]["blocks"]
    groups = []
    for block in series["plan"]:
        folder, = [p for p in matrix.attempt_paths(OUT, block) if (p / "reconciled.json").exists()]
        row = replay_block(plan, folder, block)
        assert row == read(folder / "reconciled.json")
        groups.append(dict(group=block["group"], path=folder.name, **row))
    return dict(groups=groups, qualification=plan["session_qualification"],
                clocks={lane: {key: distribution([row["clocks"][lane][key] for row in groups])
                               for key in groups[0]["clocks"][lane]} for lane in series["config"]["lanes"]},
                paired={lane: {key: dict(distribution(
                    [r["lane_minus_file_ms"][lane][key] for r in groups]),
                    file_wins=sum(r["lane_minus_file_ms"][lane][key] > 0 for r in groups))
                    for key in CLOCKS} for lane in groups[0]["lane_minus_file_ms"]},
                faster_inline_minus_file_ms={key: distribution([r["faster_inline_minus_file_ms"][key]
                                                                for r in groups]) for key in CLOCKS},
                inline_spread_ms={key: distribution([r["inline_spread_ms"][key] for r in groups])
                                  for key in CLOCKS})


def grant():
    assert (QUEUE / f"{ID}.json").read_bytes() == PACKET.read_bytes()
    latest = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+) (\S+)(?: (\S+))?",
                        (QUEUE / "STATUS.md").read_text(), re.M)[-1]
    mode, key, start, end = latest
    assert (mode, key) == ("TIMING", ID), "requires an active sole-controller grant"
    assert datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
    return dict(id=ID, start=start, end=end, request_sha256=sha(PACKET))


def timing(plan):
    active = grant()
    OUT.mkdir(parents=True)
    run.write_json(OUT / "grant.json", active)
    matrix.plan = lambda *_: iter(plan["recipe"]["blocks"])
    matrix.validate = original.validate
    matrix.replay = lambda path, _, block: replay_block(plan, path, block)
    dispatch_block = matrix.dispatch_block

    def dispatch(directory, manifest, block):
        grant()
        if not dispatch_block(directory, manifest, block):
            raise ValueError(f"block {block['group']} did not reconcile; preserve raw attempts")
        return True

    matrix.dispatch_block = dispatch
    dispatch_arm = run.dispatch_arm

    def arm(*args):
        grant()
        return dispatch_arm(*args)

    run.dispatch_arm = arm
    return matrix.dispatch(OUT, plan["configuration"], list(range(1, 6)))


if __name__ == "__main__":
    action, = sys.argv[1:]
    doc = packet()
    plan = configure()
    assert doc["blocks"] == plan["recipe"]["blocks"]
    assert (doc["input_sha256"], doc["storage"]) == (plan["recipe"]["input_sha256"], plan["recipe"]["storage"])
    assert (doc["native_host"], doc["cpus"]) == (plan["configuration"]["host"], plan["configuration"]["cpus"])
    assert (doc["native_revision"], doc["runner_sha256"]) == (plan["artifact"]["version"], parked.ELF)
    assert (doc["broker_sha256"], doc["runtime_sha256"]) == (
        plan["session_qualification"]["broker_sha256"], plan["session_qualification"]["runtime_sha256"])
    if action == "check":
        print("Verified recovered broker, unchanged clocks and peer pins, and five EXTERNAL blocks")
    elif action == "timing":
        sys.exit(timing(plan))
    elif action == "replay":
        print(json.dumps(report(plan), indent=2, sort_keys=True, allow_nan=False))
    else:
        sys.exit("expected check, timing or replay")
