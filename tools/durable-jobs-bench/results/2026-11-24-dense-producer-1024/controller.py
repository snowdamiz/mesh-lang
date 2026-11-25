"""Offline replay and sole-controller commands for the private dense producer."""
import argparse
from copy import deepcopy
from datetime import datetime, timezone
import fcntl
import json
import inspect
from pathlib import Path
import re
from statistics import median
import sys
import tarfile
from uuid import uuid4

HERE = Path(__file__).resolve().parent
ID = HERE.name
NARROW = ("128-a", "128-b")
WIDE = "1024"
EVIDENCE = HERE / "evidence"
sys.path.insert(0, str(HERE.parent / "2026-11-24-dense-product-d2-transfer/closure-retry"))
import amendment as stock

dense, matrix, run, require = stock.dense, stock.matrix, stock.run, stock.require
ROOT = dense.ROOT
PACKET = HERE / "request.json"
STAGE = HERE / "source.tar.gz"
BASE = dense.BASE
HOST_FILES = ("adapter.py", "backend.py", "identity.py", "install-writer.sql", "native-run", "wire.py")


def read_request():
    packet = run.read_json(PACKET)
    require(packet["id"] == ID and packet["input"] == dense.INPUT
            and packet["runner_elf"] == stock.ELF and packet["runner_manifest"] == stock.MANIFEST,
            "sealed scope or runner changed")
    for name, sha in packet["pins"].items():
        require(run.digest(ROOT / name) == sha, f"sealed input changed: {name}")
    require(run.digest(STAGE) == packet["source_sha256"], "source bundle changed")
    return packet


def history():
    summary = run.read_json(stock.OUTPUT / "final-replay.json")
    result = []
    for group, item in summary["blocks"].items():
        pair = {}
        for label in ("hot", "river-2x32"):
            root = stock.OUTPUT / "blocks" / item["attempt"] / f"measured-{label}"
            archive, = root.glob("*/raw.tar.gz")
            score = item["scores"][f"measured-{label}"]
            require(run.digest(archive) == score["raw_sha256"], "historical raw hash changed")
            with tarfile.open(archive) as raw:
                txn, = run.file_json(raw, "adapter/ledger.json")["transactions"]
                if label == "hot":
                    writer = run.file_json(raw, "adapter/producer-writer.json")
                    require((writer["bulk"], writer["scalar"]) == (8, 0), "historical grouped writer changed")
                    for index in range(2):
                        check_log(dense.text(raw, f"adapter/runner-{index}.log"))
            pair[label] = txn["committed_us"] - txn["submitted_us"]
        result.append(dict(group=int(group), hot_us=pair["hot"], river_us=pair["river-2x32"],
                           gap_us=pair["hot"] - pair["river-2x32"]))
    require(len(result) == 6, "historical grouped comparison incomplete")
    return result


def check_log(value):
    ready, drained = value.splitlines()
    require(ready == "READY" and json.loads(drained)["drained"], "grouped runner log is not logger-off")


def remote_stage(packet):
    return BASE / f"source-dense-producer-1024-{packet['stage_id']}"


def setup_binding(raw, packet):
    with tarfile.open(raw) as archive:
        get = dense.archive_files(archive)
        require(get("exit-status") == b"0\n" and get("SETUP-PASS") == b"", "native SETUP failed")
        setup = json.loads(get("broker.json"))
        require(setup["source_sha256"] == packet["source_sha256"] and setup["version"] == packet["revision"]
                and (setup["system"], setup["architecture"], setup["compiler_sha256"]) ==
                    ("Linux", "x86_64", dense.COMPILER_SHA)
                and setup["files"]["mesh"] != stock.ELF, "broker identity changed")
        report = json.loads(get("proof-owner/proof/result.json"))
        require(report == run.read_json(EVIDENCE / "local-proof.json"), "native proof differs from local replay")
        for width in ("128", "1024"):
            rows = json.loads(get(f"proof-owner/proof/canonical-{width}-durable.json"))
            require(len({r["receipt"]["id"] for r in rows}) == len(rows) == dense.JOBS,
                    "native proof discarded receipts")
    return setup


def stage_request(name):
    path = HERE / (name + ".json")
    request = run.read_json(path)
    require(request["id"] == f"{ID}-{name}" and request["packet_sha256"] == run.digest(PACKET),
            "stage request changed")
    return path, request


def active_grant(kind, stage):
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                        (matrix.QUEUE / "STATUS.md").read_text(), re.M)
    require(bool(events), "missing controller grant")
    mode, ident, start, end = events[-1]
    path, request = stage_request(stage)
    require((mode, ident) == (kind, request["id"]) and end
            and datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
            and (matrix.QUEUE / f"{ident}.json").read_bytes() == path.read_bytes(),
            "matching live grant and exact controller inbox bytes required")
    return dict(id=ident, start=start, end=end, request_sha256=run.digest(path))


def setup(packet):
    grant = active_grant("SETUP", "setup")
    folder = EVIDENCE / "setup" / uuid4().hex
    folder.mkdir(parents=True)
    run.write_json(folder / "grant.json", grant)
    with (dense.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        active_grant("SETUP", "setup")
        with STAGE.open("rb") as source, (folder / "raw.tar.gz").open("xb") as raw, \
             (folder / "setup.log").open("xb") as log:
            result = run.ssh(["bash", "-c", (HERE / "native-setup.sh").read_text(), "--",
                              packet["source_sha256"], packet["revision"], packet["stage_id"]],
                             stdin=source, stdout=raw, stderr=log, timeout=1100)
        run.write_json(folder / "status.json", dict(exit_code=result.returncode))
        result.check_returncode()
    setup_binding(folder / "raw.tar.gz", packet)
    print(folder)


def verify_release(name, path):
    ident = stage_request(name)[1]["id"]
    release = run.read_json(path or matrix.QUEUE / f"{ident}.released.json")
    require(release["id"] == ident and release["exit_status"] == 0, "controller stage did not release cleanly")
    return release


def stages(config, label, remote):
    if label in dense.PEERS:
        return stock.stages(config, label, remote)
    stock_config = dict(config, lanes={**config["lanes"], "hot": dict(engine="mesh", runners=2)})
    files, inventory = stock.stages(stock_config, "hot", remote)
    spec = inventory["engines"]["mesh"]
    path = config["private_stage"]
    spec["argv"] = [f"DENSE_WIDTH={'1024' if label == WIDE else '128'}" if arg == "PRODUCT_COMPOSITION=product-d2"
                    else f"{path}/packet/native-run" if arg == str(stock.OVERLAY / "native-run")
                    else arg for arg in spec["argv"]]
    spec["argv"].insert(2, f"DENSE_BROKER_SHA256={config['broker']['files']['mesh']}")
    spec["verified_files"] = {p: sha for p, sha in spec["verified_files"].items()
                              if not p.startswith(str(stock.OVERLAY) + "/")}
    spec["verified_files"].update(config["private_files"])
    run.check_inventory(inventory)
    return files, inventory


def receipt_gate(archive, get, origin, label):
    writer = get("producer-writer")
    expected = {"wide": [1, 1024]} if label == WIDE else {"narrow": [8, 1024]}
    require(writer == dict(enabled=True, bulk=0 if label == WIDE else 8, scalar=0, statements=expected),
            "actual writer calls or returned row count changed")
    rows = get("producer-receipts")
    keys = [f"job_{ix:06d}" for ix in range(1024)]
    require([r["key"] for r in rows] == keys
            and {r["key"]: r["receipt"]["id"] for r in rows} == origin["enqueued_ids"]
            and all(re.fullmatch(r"[0-9a-f]{64}", r["fingerprint"]) and r["receipt"]["inserted"]
                    and not r["receipt"]["conflict"] and r["receipt"]["state"] == "available" for r in rows),
            "durable receipts differ from all 1,024 canonical keys or broker IDs")
    for index in range(2):
        check_log(dense.text(archive, f"adapter/runner-{index}.log"))


# Reuse the full original topology, TLS, causal ledger and runner scorer.
_score_source = inspect.getsource(dense.score_arm)
begin = _score_source.index('        child_identity(record, executable')
begin = _score_source.rindex('        for record, verb', 0, begin)
end = _score_source.index('        pids =', begin)
_score_source = _score_source[:begin] + '''        child_identity(broker, config["private_stage"] + "/broker/mesh",
                       config["broker"]["files"]["mesh"], "session", launch_parent)
        for record in runners:
            child_identity(record, executable, build["files"]["mesh"], "run", launch_parent)
''' + _score_source[end:]
begin = _score_source.index('        enqueue = get("producer-writer")')
end = _score_source.index('        statements =', begin)
_score_source = _score_source[:begin] + '        receipt_gate(raw, get, origin, label)\n' + _score_source[end:]
_score_source = _score_source.replace('if label == "hot" else probes == 2 * JOBS', 'if label not in PEERS else probes == 2 * JOBS')
_scoring_scope = dict(vars(dense), stages=stages, receipt_gate=receipt_gate)
exec(compile(_score_source, dense.__file__, "exec"), _scoring_scope)
score_arm = _scoring_scope["score_arm"]


def schedules(packet, mode):
    orders = packet[mode + "_orders"]
    require(len(orders) == 2 and orders[1] == orders[0][::-1], "expected two reversed orders")
    return [dict(group=group, profile="noop", phases=["warmup", "measured"], order=labels)
            for group, labels in enumerate(orders, 1)]


def contrast(rows, packet):
    wide, a, b = (rows[label] for label in (WIDE, *NARROW))
    gate = packet["gate"]
    faster = min(a["enqueue_us"], b["enqueue_us"])
    spread = abs(a["enqueue_us"] - b["enqueue_us"])
    margin = max(gate["min_enqueue_us"], gate["min_enqueue_fraction"] * faster, spread)
    saving = faster - wide["enqueue_us"]
    drain_fast = min(a["drain_us"], b["drain_us"])
    drain_spread = abs(a["drain_us"] - b["drain_us"])
    drain_budget = max(gate["max_drain_us"], gate["max_drain_fraction"] * drain_fast, drain_spread)
    drain_loss = wide["drain_us"] - drain_fast
    return dict(qualifies=saving > margin and drain_loss <= drain_budget,
                faster_enqueue_us=faster, duplicate_enqueue_spread_us=spread,
                enqueue_saving_us=saving, minimum_saving_us=margin, drain_loss_us=drain_loss,
                maximum_drain_loss_us=drain_budget, duplicate_drain_spread_us=drain_spread)


def replay_block(path, block, config, remotes):
    require(run.read_json(path / "order.json") == block, "block order changed")
    result = {}
    for phase, label, lane, folder in matrix.paths(path, config, block):
        _, inventory = stages(config, label, run.REMOTE / remotes[label])
        score = score_arm(folder, phase, label, config, run.REMOTE / remotes[label])
        with tarfile.open(folder / "raw.tar.gz") as archive:
            txn, = run.file_json(archive, "adapter/ledger.json")["transactions"]
        result[f"{phase}-{label}"] = dict(score, enqueue_us=txn["committed_us"] - txn["submitted_us"],
            drain_us=score["last_settlement_us"] - txn["committed_us"],
            full_submit_us=score["last_settlement_us"] - txn["submitted_us"])
    return result


def summarize(directory, config, blocks, pins):
    manifest = run.read_json(directory / "series.json")
    require(all(manifest[key] == value for key, value in dict(config=config, schedule=blocks, pins=pins).items()),
            "replay binding changed")
    failures, selected, missing = {}, {}, []
    packet = read_request()
    for block in blocks:
        chosen = matrix.pick(directory, block, config, manifest["remotes"], failures)
        if not chosen:
            missing.append(block["group"])
            continue
        attempt, raw = chosen
        measured = {label: raw["measured-" + label] for label in block["order"]}
        selected[block["group"]] = dict(attempt=attempt, scores=raw, contrast=contrast(measured, packet))
    groups = list(selected.values())
    medians = {label: {clock: median(g["scores"]["measured-" + label][clock] for g in groups)
                      for clock in ("enqueue_us", "drain_us", "full_submit_us")}
               for label in blocks[0]["order"]} if groups else {}
    return dict(blocks=selected, failures=failures, missing=missing, medians=medians,
                qualifies=not missing and len(groups) == 2 and all(g["contrast"]["qualifies"] for g in groups),
                raw={str(p.relative_to(directory)): run.digest(p)
                     for p in sorted(directory.rglob("raw.tar.gz"))})


def make_config(packet, broker, mode):
    binding = stock.bind()
    config = dense.configurations(binding["original"])
    config["closure_retry"] = binding["closure_retry"]
    config["lanes"] = {label: config["lanes"][label] if label in dense.PEERS else dict(engine="mesh", runners=2)
                       for label in packet[mode + "_orders"][0]}
    path = remote_stage(packet)
    config.update(broker=broker, private_stage=str(path), private_files={
        **{str(path / "packet" / name): run.digest(HERE / name) for name in HOST_FILES},
        str(path / "broker/mesh"): broker["files"]["mesh"],
    })
    return config


def check(packet):
    for name in ("setup", "pilot", "peers"):
        stage_request(name)
    for mode in ("pilot", "peers"):
        config = make_config(packet, {"files": {"mesh": "a" * 64}}, mode)
        specs = {label: stages(config, label, run.REMOTE / ID)[1]["engines"]["mesh"]
                 for label in (*NARROW, WIDE)}
        require(specs[NARROW[0]] == specs[NARROW[1]], "duplicate controls differ")
        wide = dict(specs[NARROW[0]], argv=[arg.replace("DENSE_WIDTH=128", "DENSE_WIDTH=1024")
                                              for arg in specs[NARROW[0]]["argv"]])
        require(wide == specs[WIDE], "producer lanes differ beyond one width flag")
        if mode == "peers":
            for label in dense.PEERS:
                require(stages(config, label, run.REMOTE / ID) == stock.stages(config, label, run.REMOTE / ID),
                        "stock equal peer inventory changed")
        schedules(packet, mode)
    with tarfile.open(STAGE) as archive:
        get = dense.archive_files(archive, "packet/")
        for file in (*HOST_FILES, "link.sh", "prove.py", "fixture/main.mpl", "probe/main.mpl"):
            require(get(file) == (HERE / file).read_bytes(), f"source bundle mismatch for {file}")
    with tarfile.open(EVIDENCE / "local-proof-raw.tar.gz") as archive:
        get = dense.archive_files(archive)
        proof = json.loads(get("result.json"))
        require(proof == run.read_json(EVIDENCE / "local-proof.json")
                and proof["frozen_input_sha256"] == packet["input"], "local proof was not the frozen input")
        batch = json.loads(get("canonical.json"))
        canonical = [(j["key"], j["payload"]) for j in batch["jobs"]]
        fingerprints = []
        for width, calls in (("128", {"narrow": [8, 1024]}), ("1024", {"wide": [1, 1024]})):
            rows = json.loads(get(f"canonical-{width}-durable.json"))
            require([(r["key"], r["payload"]) for r in rows] == canonical
                    and len({r["receipt"]["id"] for r in rows}) == len(rows) == 1024
                    and proof["cases"][width]["statements"] == calls,
                    "replayed local writer calls, order, bytes or receipt IDs changed")
            fingerprints.append([r["fingerprint"] for r in rows])
        require(fingerprints[0] == fingerprints[1], "canonical fingerprints diverged")
    mock = {"128-a": {"enqueue_us": 158000, "drain_us": 1160000},
            "128-b": {"enqueue_us": 162000, "drain_us": 1170000},
            "1024": {"enqueue_us": 142000, "drain_us": 1170000}}
    require(contrast(mock, packet)["qualifies"], "material pilot control should qualify")
    for clock, value in (("enqueue_us", 151000), ("drain_us", 1200000)):
        require(not contrast(dict(mock, **{"1024": dict(mock["1024"], **{clock: value})}), packet)["qualifies"],
                "invalid pilot qualified")
    return dict(historical=history(), local_cases=len(proof["cases"]),
                input=proof["frozen_input_sha256"], pilot_orders=packet["pilot_orders"],
                peers_orders=packet["peers_orders"])


def timing_state(packet, broker, mode):
    config = make_config(packet, broker, mode)
    blocks = schedules(packet, mode)
    pins = dict(request=run.digest(PACKET), **packet["pins"])
    directory = EVIDENCE / mode
    return directory, config, blocks, pins


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("check", "setup", "bind", "timing", "replay"))
    parser.add_argument("mode", nargs="?", choices=("pilot", "peers"))
    parser.add_argument("--setup-raw", type=Path)
    parser.add_argument("--setup-release", type=Path)
    parser.add_argument("--pilot-release", type=Path)
    args = parser.parse_args()
    packet = read_request()
    if args.command == "check":
        print(json.dumps(check(packet), indent=2))
        return
    if args.command == "setup":
        setup(packet)
        return
    raw = args.setup_raw
    if raw is None:
        successful = [path / "raw.tar.gz" for path in (EVIDENCE / "setup").glob("*")
                      if (path / "status.json").is_file() and run.read_json(path / "status.json")["exit_code"] == 0]
        require(len(successful) == 1, "name the successful SETUP raw archive")
        raw = successful[0]
    broker = setup_binding(raw, packet)
    if args.command == "bind":
        print(json.dumps(broker, indent=2))
        return
    require(args.mode is not None, "specify pilot or peers")
    matrix.stages, matrix.replay_block, matrix.summarize = stages, replay_block, summarize
    timing_id = stage_request(args.mode)[1]["id"]
    matrix.grant = lambda _: active_grant("TIMING", args.mode)
    matrix.SERIES[timing_id] = timing_id
    state = timing_state(packet, broker, args.mode)
    if args.command == "replay":
        print(json.dumps(summarize(*state), indent=2))
        return
    active_grant("TIMING", args.mode)
    verify_release("setup", args.setup_release)
    if args.mode == "peers":
        verify_release("pilot", args.pilot_release)
        require(summarize(*timing_state(packet, broker, "pilot"))["qualifies"],
                "both reversed pilots must pass before comparing peers")
    return matrix.dispatch(*state[:2], timing_id, *state[2:], [1, 2])


if __name__ == "__main__":
    sys.exit(main())
