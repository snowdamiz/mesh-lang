"""Sealed build-free SLOW transfer. Only the native controller can dispatch."""

from copy import deepcopy
from datetime import datetime, timezone
from pathlib import Path
import json
import re
from statistics import median
import sys
import tarfile

from history import BASELINE, INBOX, KEYS, PEERS, ROOT, files_for
from prepare import HERE, inputs
from score import BROKER, BROKER_SHA, ELF, PROFILE, RUNNER, run, score_arm
from importlib import import_module

matrix = import_module("interleave-v2")
ID = HERE.name
PHASES = {"setup": "SETUP", "pilot": "TIMING", "peers": "TIMING"}
MESH = {"inline-a": ("inline", 1), "inline-b": ("inline", 1), "file": ("file", 1),
        "split-inline": ("inline", 2), "split-file": ("file", 2)}
PRIMARY = "split-file"
PILOT_ORDER = ("inline-a", "file", PRIMARY, "split-inline", "inline-b")
PEER_ORDERS = (("product", "river", PRIMARY, "oban", "apalis"),
               ("oban", "product", "apalis", PRIMARY, "river"),
               ("apalis", PRIMARY, "oban", "river", "product"))
STAGE = sorted((HERE / "stage").iterdir())
MINUTES = {"setup": 8, "pilot": 12, "peers": 15}


def blocks(phase):
    if phase == "setup":
        orders = (("inline-a", "file", "split-inline", PRIMARY),)
        return [dict(group=0, profile=PROFILE, phases=["diagnostic"], order=list(orders[0]))]
    if phase == "pilot":
        orders = (PILOT_ORDER, PILOT_ORDER[::-1])
    else:
        first, second, third = PEER_ORDERS
        orders = (first, first[::-1], second, second[::-1], third)
    return [dict(group=i, profile=PROFILE, phases=["warmup", "measured"], order=list(order))
            for i, order in enumerate(orders, 1)]


def grant_id(phase):
    return f"{ID}-{phase}"


def remotes(phase, config):
    return {label: f"{grant_id(phase)}-{label}" for label in config["lanes"]}


def configure(phase):
    public = run.read_json(BASELINE / "slow.inventory.json")
    config = public["configuration"]
    assert config["cpus"] == [0, 1] and config["profiles"] == ["slow"]
    recipe = public["recipe"]
    assert (recipe["product_runners"], recipe["peer_runners"], recipe["slots"],
            recipe["cap"], recipe["app_sessions"], recipe["storage"]) == (1, 1, 64, 68, 8, "default")
    stock = config["lanes"]["product"]
    assert stock["product_mode"] == "product-bulk"
    _, source = inputs()
    lanes = {}
    remote = remotes(phase, {**config, "lanes": MESH})
    for label in dict.fromkeys(label for block in blocks(phase) for label in block["order"]):
        if label not in MESH:
            lanes[label] = config["lanes"][label]
            continue
        mode, runners = MESH[label]
        lane = deepcopy(stock)
        lane.update(session_mode=mode, runners=runners)
        spec = lane["inventory"]["engines"]["mesh"]
        assert spec["verified_files"][RUNNER] == ELF
        source_path = next(Path(item).parents[5] for item in spec["argv"]
                           if item.endswith("/replay-closure-retry/native-run"))
        assert source_path.name == "source"
        home = run.REMOTE / remote[label]
        spec["argv"] = ["/bin/bash", str(home / "launch.sh"), mode,
                        "stock" if runners == 1 else "two-runner", "mesh"]
        spec["verified_files"][BROKER] = BROKER_SHA
        spec["verified_files"][str(source_path / "scripts/fixtures/jobs-role-grants.sql")] = source["role_grants"]
        spec["verified_files"].update({str(home / file.name): run.digest(file) for file in STAGE})
        lanes[label] = lane
    config["description"] = "Public SLOW same-ELF producer transport and 64-seat Mesh deployment."
    config["lanes"] = lanes
    matrix.validate(config)
    return config


def packet():
    return HERE / "controller-packet.json"


def command_check():
    doc = run.read_json(packet())
    assert doc["id"] == ID and doc["build_free"]
    for name, sha in doc["local_sha256"].items():
        assert run.digest(ROOT / name) == sha, name
    for phase in PHASES:
        assert doc["schedule"][phase] == blocks(phase)
        assert doc["configuration"][phase] == configure(phase)
    rows = [json.loads(line) for line in (HERE / "evidence/nonroot-preflight.jsonl").read_text().splitlines()]
    assert {(item["profile"][0], item["mode"]) for item in rows} == {(1, "inline"), (1, "file"), (2, "inline"), (2, "file")}
    assert all(item["uid"] != 0 and item["installed"] and not item["stage_listable"] for item in rows)
    return doc


def grant(phase, doc):
    name = grant_id(phase)
    inbox = INBOX / f"{name}.json"
    assert inbox.read_bytes() == packet().read_bytes()
    events = re.findall(r"^(?:SETUP|TIMING|RELEASED|HOLD)\b[^\n]*", (INBOX / "STATUS.md").read_text(), re.M)
    kind, actual, start, end = events[-1].split()
    assert (kind, actual) == (PHASES[phase], name)
    start_time, end_time = datetime.fromisoformat(start), datetime.fromisoformat(end)
    assert start_time <= datetime.now(timezone.utc) < end_time
    assert (end_time - start_time).total_seconds() <= 60 * doc["minutes"][phase]
    return dict(id=name, kind=kind, start=start, end=end, request_sha256=run.digest(packet()))


def replay_block(path, config, block):
    assert run.read_json(path / "order.json") == block
    phases = {}
    for phase, label, scope in files_for(path, block, config):
        phases.setdefault(phase, {})[label] = score_arm(scope, block, phase, config["lanes"][label], config)
    return phases


def measure_gates(rows, gate):
    a, b = rows["inline-a"], rows["inline-b"]
    file, split, combo = (rows[k] for k in ("file", "split-inline", PRIMARY))
    spread = {key: abs(a[key] - b[key]) for key in KEYS}
    faster = {key: min(a[key], b[key]) for key in KEYS}
    effects = {name: {key: before[key] - after[key] for key in KEYS}
               for name, before, after in (
                   ("file_at_one", faster, file), ("file_at_two", split, combo),
                   ("split_inline", faster, split), ("split_file", file, combo),
                   ("composition", faster, combo))}
    # Preserve the component effects even if the prespecified composition fails.
    checks = {}
    for name in ("file_at_one", "file_at_two"):
        effect = effects[name]
        checks[name] = (effect["enqueue_ms"] > max(gate["file_enqueue_ms"], spread["enqueue_ms"])
                        and effect["drain_ms"] >= -max(gate["transport_drain_slack_ms"], spread["drain_ms"]))
    for name in ("split_inline", "split_file"):
        checks[name] = all(effects[name][key] > max(gate["topology_ms"], spread[key])
                           for key in ("drain_ms", "last_effect_ms"))
    checks["composition"] = effects["composition"]["submit_ms"] > max(gate["composition_submit_ms"], spread["submit_ms"])
    checks["incremental_submit"] = min(file["submit_ms"], split["submit_ms"]) - combo["submit_ms"] > max(
        gate["incremental_submit_ms"], spread["submit_ms"])
    def host_cpu(row):
        return row["pinned_user_cpu_ms_per_job"] + row["pinned_kernel_cpu_ms_per_job"]
    checks["resource_brackets"] = all(
        host_cpu(row) <= gate["host_cpu_ratio"] * max(map(host_cpu, (a, b)))
        and row["runner_cpu_ms_per_job"] <= gate["runner_cpu_ratio"] * max(
            a["runner_cpu_ms_per_job"], b["runner_cpu_ms_per_job"])
        and row["peak_runner_rss_bytes"] <= gate["runner_rss_ratio"] * max(
            a["peak_runner_rss_bytes"], b["peak_runner_rss_bytes"])
        and row["peak_connections"] <= (17 if "split" in name else 13)
        for name, row in (("file", file), ("split-inline", split), (PRIMARY, combo)))
    return dict(passed=all(checks.values()), checks=checks, duplicate_spread_ms=spread, effects_ms=effects)


def report(phase, doc):
    out = HERE / "evidence/native" / phase
    release = run.read_json(out / "controller-release.json")
    expected = run.read_json(out / "grant.json")
    assert (release["id"], release["exit_status"], release["grant"]) == (grant_id(phase), 0, expected)
    assert expected["request_sha256"] == run.digest(packet())
    cfg = doc["configuration"][phase]
    series = run.read_json(out / "series.json")
    assert series["config"] == cfg and series["plan"] == blocks(phase)
    evidence = []
    for block in series["plan"]:
        path, = [p for p in matrix.attempt_paths(out, block) if (p / "reconciled.json").exists()]
        raw = replay_block(path, cfg, block)
        assert raw == run.read_json(path / "reconciled.json")
        evidence.append(dict(group=block["group"], path=path.name, scores=raw))
    result = dict(phase=phase, groups=evidence)
    if phase == "pilot":
        gates = [measure_gates(g["scores"]["measured"], doc["gate"]) for g in evidence]
        result.update(gates=gates, passed=all(g["passed"] for g in gates))
    if phase == "peers":
        pairs = {}
        for label in ("product", *PEERS):
            pairs[label] = {}
            for key in KEYS:
                deltas = [g["scores"]["measured"][label][key] - g["scores"]["measured"][PRIMARY][key]
                          for g in evidence]
                pairs[label][key] = dict(blocks=deltas, median=median(deltas), wins=sum(d > 0 for d in deltas))
        result["composed_mesh_vs_stock_peers"] = pairs
    return result


def dispatch(phase, doc):
    if phase != "setup":
        report("setup", doc)
    if phase == "peers":
        assert report("pilot", doc)["passed"], "the prespecified pilot did not qualify"
    active = grant(phase, doc)
    output = HERE / "evidence/native" / phase
    output.mkdir(parents=True)
    run.write_json(output / "grant.json", active)
    config = doc["configuration"][phase]
    series = dict(config=config, plan=blocks(phase), remote=remotes(phase, config),
                  drivers={path.name: run.digest(path) for path in (
                      Path(matrix.__file__), run.SUITE / "run.py", run.SUITE / "verify.py", run.SUITE / "worker.py")})
    run.write_json(output / "series.json", series)
    matrix.plan = lambda *_: iter(series["plan"])
    matrix.replay = replay_block
    matrix.ATTEMPT_ERRORS += (AssertionError,)
    stage_file = run.stage

    def stage_with_adapter(path, remote, log):
        grant(phase, doc)
        stage_file(path, remote, log)
        if path.name == "inventory.json" and path.parent.name in MESH:
            for file in STAGE:
                stage_file(file, remote, log)

    run.stage = stage_with_adapter
    original_dispatch = matrix.dispatch_block

    def dispatch_block(directory, manifest, block):
        grant(phase, doc)
        if not original_dispatch(directory, manifest, block):
            raise ValueError(f"group {block['group']} failed reconciliation")
        return True

    matrix.dispatch_block = dispatch_block
    return matrix.dispatch(output, config, None)


if __name__ == "__main__":
    action, *rest = sys.argv[1:]
    doc = command_check()
    if action == "check" and not rest:
        print("Verified three sealed configurations, stage pins, and four nonroot no-workload preflights.")
    else:
        phase, = rest
        if action == "dispatch":
            sys.exit(dispatch(phase, doc))
        if action == "replay":
            print(json.dumps(report(phase, doc), indent=2, sort_keys=True, allow_nan=False))
        else:
            sys.exit("expected check, dispatch PHASE, or replay PHASE")
