"""Fence, stage and replay the separate dense D2 and completion-writer series."""
import argparse
from copy import deepcopy
from datetime import datetime, timezone
import importlib
from itertools import permutations
import json
from pathlib import Path
import re
from statistics import mean, median
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
EQUAL = HERE.parent / "2026-11-23-dense-matched-topology"
D2 = HERE.parent / "2026-11-24-exit-confirmed-bulk-d2"
BULK = HERE.parent / "2026-11-23-bulk-enqueue-v2-short"
ID = HERE.name
BASE = "/var/lib/durable-jobs-bench/native"
R2 = f"{BASE}/builds/bulk-writer-b1dcd34af288"
V2 = f"{BASE}/builds/bulk-enqueue-3b09dc3d0e23-ordinary-v2"
V1 = f"{BASE}/builds/ordinary-trusted-v1-23fdb3f5530d"
POSTGRES = f"{BASE}/toolchain/pgsql-17.11/bin/postgres"
DRIVERS = ("native-dense.sh", "native-arm.py", "identity.py")
PLAN = json.loads((HERE / "native-plan.json").read_text())
PEERS = ("river-2x32", "oban-2x32", "apalis-2x32")

sys.path.insert(0, str(EQUAL))
import matrix
from matrix import CPUS, INPUT, QUEUE, ROOT, SUITE, require, run

native_score = importlib.import_module("native-score")
base_stages = matrix.stages
base_score = matrix.score_arm
RESULTS = HERE / "evidence/native"
REQUEST = HERE / "native-request.json"


def d2_source():
    path = D2 / "replay"
    sys.path.insert(0, str(path))
    try:
        controller = importlib.import_module("controller")
    finally:
        sys.path.remove(str(path))
    require(Path(controller.__file__).parent == path, "loaded the wrong D2 proof driver")
    return controller


def request():
    packet = run.read_json(REQUEST)
    require(packet["id"] == ID and packet["input_sha256"] == INPUT and packet["cpus"] == CPUS
            and packet["controller"] == "linode-native-stepchange-controller-v1", "wrong controller request")
    for path, expected in packet["local_sha256"].items():
        require(run.digest(ROOT / path) == expected, f"changed local source or proof: {path}")
    return packet


def schedule():
    labels = {*native_score.MODES, *PEERS}
    require(PLAN["input_sha256"] == INPUT and PLAN["profile"] == "noop"
            and PLAN["phases"] == ["warmup", "measured"], "unexpected fixture or phases")
    for contrast in ("d2", "writer"):
        triplet = PLAN["separate"][contrast]
        orders = [tuple(label for label in order if label in triplet) for order in PLAN["orders"]]
        require(len(orders) == 6 and set(orders) == set(permutations(triplet)), "contrast rotation set changed")
    for index, order in enumerate(PLAN["orders"], 1):
        require(len(order) == len(labels) and set(order) == labels, "lane order changed")
        yield dict(group=index, profile="noop", phases=PLAN["phases"], order=order)


def seal(spec, path):
    return dict(path=path, elf=spec["verified_files"][path + "/mesh"],
                manifest=spec["verified_files"][path + "/mesh.json"])


def configurations(d2):
    equal = run.read_json(EQUAL / "equal.json")
    matrix.import_equal().check_config(equal)
    bulk = run.read_json(BULK / "inventory.json")
    require(all(equal[k] == bulk[k] for k in ("host", "cpus", "clock_ticks")), "native pins disagree")
    build_v1 = equal["inventory"]["engines"]["mesh"]
    build_bulk = bulk["lanes"]["bulk"]["inventory"]["engines"]["mesh"]
    source = run.read_json(D2 / "controller-packet.json")
    cold = source["native_retained"]
    require(cold["path"] == R2, "cold r2 artifact path changed")
    r2 = dict(path=R2, elf=cold["elf_sha256"], manifest=cold["manifest_sha256"])
    v1, v2 = seal(build_v1, V1), seal(build_bulk, V2)
    equal.update(description="Dense D2 and completion-writer crossover, V1 bridge, and equal 2x32 stock peers",
                 mesh_broker={}, mesh_runner={}, mesh_spec={},
                 postgres_sha256=source["native_cold_pins"]["files"][POSTGRES])
    equal["lanes"] = {label: equal["lanes"][label] for label in PEERS}
    for label, mode in native_score.MODES.items():
        is_v1 = mode == "v1"
        equal["lanes"][label] = dict(engine="mesh", runners=2)
        equal["mesh_broker"][label] = v1 if is_v1 else v2
        equal["mesh_runner"][label] = v1 if is_v1 else (d2 if mode.startswith("d2") else r2)
        equal["mesh_spec"][label] = build_v1 if is_v1 else build_bulk
    return equal


def stages(config, label, remote):
    if label in PEERS:
        files, out = base_stages(config, label, remote)
        out["engines"][config["lanes"][label]["engine"]]["verified_files"][POSTGRES] = config["postgres_sha256"]
        run.check_inventory(out)
        return files, out
    spec = deepcopy(config["mesh_spec"][label])
    spec.pop("run_as", None)
    mode, runner = native_score.MODES[label], config["mesh_runner"][label]
    files = [SUITE / "worker.py", *(HERE / name for name in DRIVERS)]
    pins = (runner["elf"], runner["manifest"]) if mode.startswith("d2") else ("unused", "unused")
    spec["argv"] = ["/bin/bash", str(remote / "native-dense.sh"), mode, runner["path"], *pins, "mesh"]
    spec["evidence_globs"] = ["*.json", "*.log", "*.txt", "*.jsonl"]
    spec["verified_files"].update({
        str(remote / path.name): run.digest(path) for path in files
    } | {runner["path"] + "/mesh": runner["elf"], runner["path"] + "/mesh.json": runner["manifest"],
         POSTGRES: config["postgres_sha256"]})
    out = dict(config["inventory"], engines={**config["inventory"]["engines"], "mesh": spec})
    run.check_inventory(out)
    return files, out


def grant(_=None):
    events = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD)\s+(\S+)\s+(\S+)\s+(\S+)",
                        (QUEUE / "STATUS.md").read_text(), flags=re.M)
    require(events and events[-1][:2] == ("TIMING", ID), f"no active controller TIMING assignment for {ID}")
    _, _, start, end = events[-1]
    require(datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end),
            "outside the assigned timing window")
    require((QUEUE / f"{ID}.json").read_bytes() == REQUEST.read_bytes(), "controller request changed")
    return dict(start=start, end=end, request_sha256=run.digest(REQUEST))


def score_arm(folder, phase, label, config, remote):
    _, inventory = stages(config, label, remote)
    if label not in PEERS:
        return native_score.raw_score(folder, phase, label, config, inventory)
    score = base_score(folder, config["lanes"][label]["engine"], 2, phase, inventory, config)
    with tarfile.open(folder / "raw.tar.gz", "r:gz") as raw:
        ledger = run.file_json(raw, "adapter/ledger.json")
    return dict(score, last_effect_us=max(j["effect_committed_us"] for j in ledger["jobs"]))


def replay_block(path, block, config, remotes):
    require(run.read_json(path / "order.json") == block, "attempt differs from preregistered order")
    return {f"{phase}-{label}": score_arm(folder, phase, label, config, run.REMOTE / remotes[label])
            for phase, label, _, folder in matrix.paths(path, config, block)}


def metrics(row):
    terminal, commit, submit = (row[key] for key in ("last_settlement_us", "first_commit_us", "first_submit_us"))
    return dict(submit_ms=(terminal - submit) / 1000, drain_ms=(terminal - commit) / 1000,
                effect_ms=(row["last_effect_us"] - commit) / 1000,
                enqueue_ms=(commit - submit) / 1000, runner_cpu_ms_per_job=row["runner_cpu_ms_per_job"])


def contrast(arms, labels):
    a, candidate, b = (metrics(arms[label]) for label in labels)
    floor = PLAN["advancement"]["minimum_each_clock_percent"] / 100
    rows = {}
    for key in a:
        faster = min(a[key], b[key])
        control = mean((a[key], b[key]))
        rows[key] = dict(elapsed_saving_vs_faster=faster - candidate[key],
                         elapsed_saving_vs_mean=control - candidate[key],
                         control_spread=abs(a[key] - b[key]),
                         meaningful=faster - candidate[key] > max(abs(a[key] - b[key]), floor * faster))
        if key in PLAN["primary"]:
            rows[key]["throughput_change_vs_mean_percent"] = 100 * (control / candidate[key] - 1)
    return dict(metrics=rows, mechanism={label: {k: arms[label][k] for k in (
                    "checkout_probes", "terminal_xids", "max_terminal_cohort", "admitted_queries",
                    "probe_sql_exec_ms", "application_sql_exec_ms")} for label in labels},
                qualifies=all(rows[key]["meaningful"] for key in PLAN["primary"])
                           and rows["effect_ms"]["elapsed_saving_vs_faster"] > 0)


def dist(values):
    return dict(min=min(values), median=median(values), max=max(values))


def summarize(directory, config, blocks, pins):
    manifest = run.read_json(directory / "series.json")
    require(manifest["config"] == config and manifest["schedule"] == blocks and manifest["pins"] == pins,
            "series pins changed")
    failures, selected, missing = {}, {}, []
    for block in blocks:
        group = block["group"]
        found = matrix.pick(directory, block, config, manifest["remotes"], failures)
        if found is None:
            missing.append(group)
            continue
        attempt, scores = found
        arms = {label: scores["measured-" + label] for label in block["order"]}
        normalized = {label: metrics(row) for label, row in arms.items()}
        direct = {name: contrast(arms, PLAN["separate"][name]) for name in ("d2", "writer")}
        comparisons = {}
        for candidate in ("d2-on", "r2-zero"):
            comparisons[candidate] = {peer: {key: normalized[peer][key] - normalized[candidate][key]
                                      for key in ("submit_ms", "drain_ms", "effect_ms")}
                                      for peer in (native_score.V1, *PEERS)}
        selected[group] = dict(attempt=attempt, raw_scores=scores, direct=direct, peers_and_v1=comparisons,
            r2_off_vs_d2_off={key: mean(normalized[label][key] for label in ("r2-off-a", "r2-off-b"))
                             - mean(normalized[label][key] for label in ("d2-off-a", "d2-off-b"))
                             for key in ("submit_ms", "drain_ms", "effect_ms")})
    summary = {}
    if selected:
        for name in ("d2", "writer"):
            reports = [row["direct"][name] for row in selected.values()]
            first = reports[0]["metrics"]
            values = {key: {delta: dist([row["metrics"][key][delta] for row in reports])
                            for delta in first[key] if delta != "meaningful"} for key in first}
            wins = sum(report["qualifies"] for report in reports)
            controls = PLAN["separate"][name]
            signal = "checkout_probes" if name == "d2" else "terminal_xids"
            mechanism = [tuple(row["mechanism"][label][signal] for label in controls) for row in reports]
            reduction = [min(a, b) - candidate for a, candidate, b in mechanism]
            spread = [abs(a - b) for a, _, b in mechanism]
            summary[name] = dict(metrics=values, meaningful_rotations=wins, advances=not missing
                                 and wins >= PLAN["advancement"]["minimum_qualified_rotations"],
                                 mechanism=dict(counter=signal, candidate_count=dist([row[1] for row in mechanism]),
                                   reduction_vs_lower_control=dist(reduction), control_spread=dist(spread),
                                   rotations_exceeding_spread=sum(n > noise for n, noise in zip(reduction, spread))))
        summary["peers_and_v1"] = {
            candidate: {peer: {key: dist([r["peers_and_v1"][candidate][peer][key] for r in selected.values()])
                               for key in ("submit_ms", "drain_ms", "effect_ms")}
                        for peer in (native_score.V1, *PEERS)}
            for candidate in ("d2-on", "r2-zero")}
    return dict(qualified_rotations=len(selected), missing=missing, failures=failures, blocks=selected, summary=summary)


def attach_matrix():
    matrix.grant = grant
    matrix.stages = stages
    matrix.replay_block = replay_block
    matrix.summarize = summarize
    matrix.SERIES[ID] = ID


def dispatch(config, blocks, pins, groups):
    grant()
    if any(g > 2 for g in groups):
        prior = summarize(RESULTS, config, blocks, pins)
        require(not {1, 2} & set(prior["missing"]), "both pilot rotations must reconcile before expanding")
    underlying = run.dispatch_arm

    def dispatch_checked(path, remote, block, phase, engine):
        underlying(path, remote, block, phase, engine)
        folder = path / f"{phase}-{engine}"
        outcome = run.read_json(folder / "dispatch.json")
        require(outcome.get("exit_code") == 0, f"{path.name}: controller must reconcile dispatch {outcome}")
        label = path.name.removeprefix(phase + "-")
        score_arm(folder, phase, label, config, remote)

    run.dispatch_arm = dispatch_checked
    if not RESULTS.exists():
        RESULTS.parent.mkdir(parents=True, exist_ok=True)
    matrix.dispatch(RESULTS, config, ID, blocks, pins, groups)
    summary = summarize(RESULTS, config, blocks, pins)
    return bool(set(groups) & set(summary["missing"]))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "timing", "replay"))
    parser.add_argument("groups", type=int, choices=range(1, 7), nargs="*")
    args = parser.parse_args()
    packet = request()
    blocks = list(schedule())
    d2 = d2_source()
    prior = d2.pins()
    if args.action == "check":
        # This sentinel validates the inventory shape; only current regular,
        # archived SETUP receipts can supply the real D2 ELF and manifest.
        candidate = dict(path=f"{BASE}/builds/exit-bulk-d2-000000000000", elf="0" * 64, manifest="0" * 64)
        config = configurations(candidate)
        for label in config["lanes"]:
            stages(config, label, run.REMOTE / label)
        subprocess.run(["bash", "-n", HERE / "native-dense.sh"], check=True)
        proof = d2.earlier_proof()
        print(json.dumps(dict(request=packet["id"], rotations=blocks, lanes=len(config["lanes"]),
                              warmup_and_measured_arms=len(config["lanes"]) * len(blocks) * 2,
                              archived_r2_proof=proof["first_archive"], d2_source_sha256=prior["source_archive_sha256"],
                              needs_native_d2_setup=not (D2 / "evidence/native-qualification.json").exists()),
                         indent=2))
        return 0
    if args.action == "timing":
        grant()
        require(bool(args.groups) and len(args.groups) == len(set(args.groups)), "specify distinct granted rotations")
    previous = d2.qualified(prior)
    d2_seal = previous["lanes"]["r2-d2"]["runner_seal"]
    argv = previous["lanes"]["r2-d2"]["inventory"]["engines"]["mesh"]["argv"]
    assignment, = [arg for arg in argv if arg.startswith("COMBINED_RUNNER_ARTIFACTS=")]
    candidate = dict(path=assignment.split("=", 1)[1], elf=d2_seal["elf"], manifest=d2_seal["manifest"])
    config = configurations(candidate)
    pins = {"native_request": run.digest(REQUEST), "d2_setup":
            run.digest(D2 / "evidence/native-qualification.json"), **packet["local_sha256"]}
    attach_matrix()
    if args.action == "timing":
        return dispatch(config, blocks, pins, args.groups)
    report = summarize(RESULTS, config, blocks, pins)
    print(json.dumps(report, indent=2, sort_keys=True, allow_nan=False))
    return bool(report["missing"])


if __name__ == "__main__":
    sys.exit(main())
