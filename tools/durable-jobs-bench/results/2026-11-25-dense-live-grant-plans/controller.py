"""Offline checks and replay, with distinct entry points for the sole controller."""
from collections import Counter
from copy import deepcopy
from datetime import datetime, timezone
import fcntl
import gzip
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import time

from launcher import rewrite
from plans import reduce

HERE = Path(__file__).resolve().parent
RESULTS = HERE.parent
SUITE = RESULTS.parent
ROOT = SUITE.parent.parent
ID = HERE.name
PACKET = HERE / "controller-packet.json"
QUEUE = ROOT / "target/linode-jobs-stepchange/requests"
OUT = HERE / "evidence/native"
TRANSFER = RESULTS / "2026-11-24-dense-product-d2-transfer"
BASE = TRANSFER / "evidence/native-closure-retry"
CLAIMS = RESULTS / "2026-11-24-public-dense-set-claim"
SEED = RESULTS / "2026-11-24-public-dense-bounded-grant/evidence/native"
PRODUCT = RESULTS / "2026-11-24-trusted-jobs-product-composition/source.tar.gz"
BATCH = CLAIMS / "evidence/batch_000000.json.gz"
HOST = ["90ec1c24-08c8-4bc1-aa28-79b6db1bd13a", "6.8.0-149-generic", "8:0"]
INPUT = "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"
MODULE = "/var/lib/durable-jobs-bench/native/dense-live-grant-plans-20261125/auto_explain.so"
LANES = ("off-a", "sampled", "off-b")
BLOCKS = [dict(group=i, profile="noop", phases=["measured"], order=list(order))
          for i, order in enumerate((LANES, LANES[::-1]), 1)]
ARM_FILES = ("launch.sh", "launcher.py", "observe.py", "plans.py", "settings.py")
SETUP_FILES = ("auto_explain.c", "canary.py", "plans.py", "settings.py")

sys.path[:0] = [str(SUITE), str(TRANSFER)]
import run
import transfer

need = run.require


def packet():
    data = run.read_json(PACKET)
    need(data["id"] == ID and data["schedule"] == BLOCKS and data["input_sha256"] == INPUT
         and data["native_host"] == HOST, "request binding drift")
    for name, digest in data["local_sha256"].items():
        need(run.digest(ROOT / name) == digest, f"sealed input changed: {name}")
    return data


def setup_receipt():
    receipt = run.read_json(OUT / "setup.json")
    canary = receipt["canary"]
    need(receipt["remote"] == MODULE and re.fullmatch("[0-9a-f]{64}", receipt["sha256"])
         and canary["postgres"] == "17.11" and canary["scoped_role"] == "parity_subject"
         and canary["unrelated_sql_ran_without_plan"]
         and {r["toplevel"] for r in canary["nested_query_keys"]} == {False, True},
         "invalid SETUP proof")
    return receipt


def configuration(module_hash):
    config = run.read_json(BASE / "series.json")["config"]
    original = run.read_json(BASE / "staging/hot/inventory.json")
    need(config["host"] == HOST and config["cpus"] == [0, 1] and config["profiles"] == ["noop"],
         "historical host/topology changed")
    config["lanes"] = {}
    for lane in LANES:
        inventory = deepcopy(original)
        spec = inventory["engines"]["mesh"]
        argv = spec["argv"]
        stage = run.REMOTE / f"{ID}-{lane}"
        need(argv[-1] == "mesh" and Path(argv[-2]).name == "native-run"
             and "PRODUCT_COMPOSITION=product-d2" in argv and "PRODUCT_TOPOLOGY=two-runner" in argv
             and "PRODUCT_STORAGE=default" in argv, "hot public launch drift")
        pins = {p: v for p, v in spec["verified_files"].items()
                if not (p.startswith(str(run.REMOTE) + "/") and p.endswith("/worker.py"))}
        pins[str(stage / "worker.py")] = run.digest(SUITE / "worker.py")
        pins.update({str(stage / name): run.digest(HERE / name) for name in ARM_FILES})
        if lane == "sampled":
            pins[MODULE] = module_hash
        spec.update(argv=[*argv[:-2], "/bin/bash", str(stage / "launch.sh"), argv[-2], lane,
                          MODULE if lane == "sampled" else "-", "mesh"], verified_files=pins,
                    evidence_globs=["*.json", "runner-*.log", "broker-*.log", "enqueue-*.log"])
        config["lanes"][lane] = dict(engine="mesh", runners=2, inventory=inventory)
        run.check_inventory(inventory)
    return config


def grant(kind):
    registered = QUEUE / f"{ID}.json"
    need(registered.is_file() and registered.read_bytes() == PACKET.read_bytes(),
         "sole controller has not registered these request bytes")
    name = f"{ID}-{kind.lower()}"
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+)(?: (\S+))?",
                        (QUEUE / "STATUS.md").read_text(), re.M)
    mode, reservation, start, end = events[-1]
    need((mode, reservation) == (kind, name)
         and datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end),
         f"requires active controller {kind} grant {name}")
    return dict(id=name, start=start, end=end, request_sha256=run.digest(PACKET))


def left(active):
    return int((datetime.fromisoformat(active["end"]) - datetime.now(timezone.utc)).total_seconds())


def setup():
    active = grant("SETUP")
    need(left(active) >= 100, "SETUP window too short")
    OUT.mkdir(parents=True, exist_ok=True)
    run.write_json(OUT / "setup-grant.json", active)
    archive = io.BytesIO()
    with tarfile.open(fileobj=archive, mode="w") as bundle:
        for name in SETUP_FILES:
            bundle.add(HERE / name, arcname=name)
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        with (OUT / "setup.log").open("xb") as log:
            reply = run.ssh(["timeout", "--signal=INT", "--kill-after=10s", "85s",
                             "bash", "-c", (HERE / "setup.sh").read_text()],
                            timeout=99, input=archive.getvalue(), stdout=subprocess.PIPE, stderr=log)
    (OUT / "setup.stdout").write_bytes(reply.stdout)
    reply.check_returncode()
    run.write_json(OUT / "setup.json", json.loads(reply.stdout))
    setup_receipt()


def historical_score(config):
    original = run.read_json(BASE / "staging/hot/inventory.json")
    folder = sorted(BASE.glob("blocks/*/measured-hot/measured-mesh/raw.tar.gz"))[0].parent
    return score_stock(folder, config, original)


def score_stock(folder, config, inventory):
    original = transfer.stages
    try:
        transfer.stages = lambda *_: ([], inventory)
        return transfer.score_arm(folder, "measured", "hot",
                                  dict(config, lanes={"hot": dict(engine="mesh", runners=2)}), None)
    finally:
        transfer.stages = original


def score_arm(folder, config, lane, receipt):
    score = score_stock(folder, config, config["lanes"][lane]["inventory"])
    with tarfile.open(folder / "raw.tar.gz") as archive:
        def read(name):
            return run.file_json(archive, f"adapter/{name}.json")
        settings = read("grant-observer")
        frames, closed = read("grant-frames"), read("grant-closed")
        group = read("queue-policy")
        sampled = lane == "sampled"
        payload = read("grant-plans") if sampled else []
        need(settings["lane"] == lane and
             settings["module_sha256"] == (receipt["sha256"] if sampled else None),
             "observer/module drift")
        need(("adapter/grant-plans.json" in archive.getnames()) == sampled, "unexpected plan artifact")
        need(len(frames) == 3 and len(frames[0]["runner_ports"]) == 2
             and frames[1]["end_us"] < score["first_submit_us"]
             and score["last_settlement_us"] < frames[2]["us_epoch"] <= closed["us_epoch"]
             and not [s for s in closed["sessions"] if s["usename"] in
                      ("parity_subject", "parity_producer", "compiled_caller")],
             "scope, clock or drain proof changed")
        proof = reduce(payload, frames, closed, score)
        need(proof["relations"][1]["n_tup_ins"] == 0 and proof["relations"][-1]["n_tup_ins"] == 1024
             and proof["relations"][-1]["n_tup_upd"] == 2048
             and group["completion_cohort_queries"] > 0 and score["checkout_probes"] < 32,
             "fresh relation, grant/settlement churn or hot checkout was not proved")
        if sampled:
            active = [s for s in proof["samples"] if s["window"] == "drain"]
            kinds = Counter(s["category"] for s in active)
            expected = {"claim", "grant", "recover_due", "recover_expired", "group_update", "scalar_update"}
            need(expected <= kinds.keys(), f"missing live plan classes: {sorted(expected - kinds.keys())}")
            need(any(s["category"] == "grant" and any(n.get("Relation Name") == "jobs" and n["Actual Loops"] > 0
                                                     for n in s["nodes"]) for s in active),
                 "live jobs-table grant nodes absent")
            nested = {"grant", "recover_due", "recover_expired", "group_update", "scalar_update"}
            for sample in active:
                if sample["category"] in nested:
                    need(sample["statement_keys"][0]["toplevel"] is False,
                         "nested plan matched a top-level statement")
            grant_pids = {s["pid"] for s in active if s["category"] == "grant"}
            known = list(map(set, proof["participants"].values()))
            need(all(grant_pids & pids for pids in known), "both runner origins lack live sampled grants")
        return dict(score, observation=proof)


def replay_block(folder, block, config, receipt):
    need(run.read_json(folder / "order.json") == block, "rotation order changed")
    arms = {lane: score_arm(folder / lane / "measured-mesh", config, lane, receipt)
            for lane in block["order"]}
    overhead = {}
    for metric, begin, end in (
        ("submit_ms", "first_submit_us", "last_settlement_us"),
        ("drain_ms", "first_commit_us", "last_settlement_us"),
    ):
        values = [(row[end]-row[begin])/1000 for row in (arms[lane] for lane in LANES)]
        a, sampled, b = values
        overhead[metric] = dict(off_a=a, sampled=sampled, off_b=b,
                                sampled_minus_off_mean=sampled-(a+b)/2, duplicate_spread=abs(a-b))
    return dict(overhead=overhead, arms=arms)


def timing():
    active = grant("TIMING")
    receipt = setup_receipt()
    need(re.search(rf"^RELEASED {re.escape(ID)}-setup .*exit 0\b",
                   (QUEUE / "STATUS.md").read_text(), re.M), "SETUP has no clean controller release")
    config = configuration(receipt["sha256"])
    run.write_json(OUT / "timing-grant.json", active)
    run.write_json(OUT / "series.json", dict(config=config, schedule=BLOCKS))
    host = """
set -eu
exec 8</var/tmp/mesh-durable-jobs-matrix/active-arm.lock
flock -n 8
exec 9</var/lib/durable-jobs-bench/native/native.lock
flock -n 9
test "$(cat /proc/sys/kernel/random/boot_id)" = 90ec1c24-08c8-4bc1-aa28-79b6db1bd13a
test "$(uname -smr)" = "Linux 6.8.0-149-generic x86_64"
test "$(findmnt -nro MAJ:MIN --target /var/lib/durable-jobs-bench/native)" = 8:0
"""
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        with (OUT / "timing-preflight.log").open("xb") as log:
            run.ssh(["bash", "-c", host], stdout=log, stderr=log, timeout=12).check_returncode()
        for lane in LANES:
            stage = run.REMOTE / f"{ID}-{lane}"
            local = OUT / "staging" / lane
            local.mkdir(parents=True)
            inventory = local / "inventory.json"
            run.write_json(inventory, config["lanes"][lane]["inventory"])
            with (local / "stage.log").open("xb") as log:
                for path in [*(HERE / f for f in ARM_FILES), SUITE/"worker.py", inventory]:
                    grant("TIMING")
                    run.stage(path, stage, log)
        for block in BLOCKS:
            group = OUT / "blocks" / run.block_prefix(block)
            group.mkdir(parents=True)
            run.write_json(group / "order.json", block)
            for lane in block["order"]:
                need(left(grant("TIMING")) >= 130, "insufficient booked time for next arm")
                folder = group / lane
                folder.mkdir()
                ssh = run.ssh
                def within_grant(args, *, timeout, **options):
                    remaining = left(grant("TIMING")) - 3
                    need(remaining > 0, "TIMING window ended")
                    return ssh(args, timeout=min(timeout, remaining), **options)
                run.ssh = within_grant
                try:
                    run.dispatch_arm(folder, run.REMOTE / f"{ID}-{lane}", block, "measured", "mesh")
                finally:
                    run.ssh = ssh
                need(run.read_json(folder / "measured-mesh/dispatch.json").get("exit_code") == 0,
                     f"{lane} failed; raw attempt held for controller")
            run.write_json(group / "reconciled.json", replay_block(group, block, config, receipt))


def check():
    config = configuration("a"*64)
    for shell in ("setup.sh", "launch.sh"):
        subprocess.run(["bash", "-n", HERE / shell], check=True)
    for code in HERE.glob("*.py"):
        compile(code.read_text(), str(code), "exec")
    with tarfile.open(PRODUCT) as bundle:
        name, = (n for n in bundle.getnames() if n.endswith("trusted-jobs-product-composition/replay/native-run"))
        old = bundle.extractfile(name).read()
        for lane in LANES:
            argv = config["lanes"][lane]["inventory"]["engines"]["mesh"]["argv"]
            need(hashlib.sha256(old).hexdigest() == config["lanes"][lane]["inventory"]["engines"]["mesh"]
                 ["verified_files"][argv[-4]], "pinned launcher bytes changed")
            subprocess.run(["bash", "-n"], input=rewrite(old.decode(), Path(argv[-4]).parents[5], lane, argv[-2]),
                           text=True, check=True)
    raw = gzip.decompress(BATCH.read_bytes())
    need(hashlib.sha256(hashlib.sha256(raw).hexdigest().encode()).hexdigest() == INPUT
         and len(json.loads(raw)["jobs"]) == 1024
         and all(len(job["payload"].encode()) == 1024 for job in json.loads(raw)["jobs"]),
         "frozen producer data changed")
    seed = run.read_json(SEED / "plan-gate.json")
    need(seed["passed"] is False and not seed["checks"]["update_visits"], "negative seed gate changed")
    prior = historical_score(config)
    need(prior["completed"] == 1024 and prior["checkout_probes"] == 4, "public hot baseline drift")
    print(json.dumps(dict(archived_hot_raw_sha256=prior["raw_sha256"],
                          local_canary=run.read_json(HERE / "evidence/local-canary.json"),
                          seed_negative=True, schedule=BLOCKS), indent=2))


def replay():
    receipt = setup_receipt()
    config = configuration(receipt["sha256"])
    release = run.read_json(OUT / "controller-release.json")
    need(release["id"] == f"{ID}-timing" and release["exit_status"] == 0, "TIMING has no clean release")
    groups = [replay_block(OUT / "blocks" / run.block_prefix(block), block, config, receipt)
              for block in BLOCKS]
    print(json.dumps(groups, indent=2))


if __name__ == "__main__":
    actions = {"check": check, "setup": setup, "timing": timing, "replay": replay}
    if len(sys.argv) != 2 or sys.argv[1] not in actions:
        raise SystemExit("usage: controller.py check|setup|timing|replay")
    packet()
    actions[sys.argv[1]]()
