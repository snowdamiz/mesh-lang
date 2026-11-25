"""Dispatch and replay five fresh, canonical eight-runner peer rotations."""
import argparse
import ast
from datetime import datetime, timezone
import fcntl
from hashlib import sha256
import json
from pathlib import Path
import runpy
from statistics import median
import subprocess
import sys
import tarfile
from types import ModuleType

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent
REPO = HERE.parents[4]
sys.path.insert(0, str(PARENT))
ID = "2026-11-25-public-dense-late-capacity-fresh-peers"
OLD = "893710708"
PILOT = PARENT / "evidence/pilot"
BOUND = PARENT / "evidence/bound-pilot"
PROOF = PARENT / "evidence/native-proof"
PACKET = HERE / "controller-packet.json"
STAGED = HERE / "staging"
STOCK = HERE / "stock"
ENGINES = dict(river="go", oban="oban", apalis="rust")
HOST = ["90ec1c24-08c8-4bc1-aa28-79b6db1bd13a", "6.8.0-149-generic", "8:0"]
ELF = "839c4f32eb4bff7559a3d8f76d959f8ae4e0469b86a2db69fca4cc1723c26c1b"
ORDERS = (
    ("A1", "river", "B", "oban", "reference", "apalis", "A2"),
    ("A2", "apalis", "reference", "oban", "B", "river", "A1"),
    ("river", "reference", "A2", "B", "apalis", "A1", "oban"),
    ("oban", "A1", "apalis", "B", "A2", "reference", "river"),
    ("reference", "A2", "oban", "river", "A1", "B", "apalis"),
)
SCHEDULE = [dict(group=i, order=list(order), phases=["warmup", "measured"], profile="noop")
            for i, order in enumerate(ORDERS, 1)]
LABELS = ("A1", "B", "reference", *ENGINES)
RC = "path-recovery/peer-stage-recovery/reverse-continuation"
CLOCKS = ("submit_us", "drain_us", "last_effect_us", "last_effect_from_submit_us")
RESOURCES = ("runner_cpu_ms", "pg_cpu_ms", "host_cpu_ms", "peak_connections", "bounded_calls",
             "positive_grants", "peer_sql_calls")


def released_basis():
    packet, qualified = pilot.checked_binding(BOUND)
    assert qualified["elf_sha256"] == ELF
    release = pilot.run.read_json(PILOT / "controller-release.json")
    assert release["id"] == packet["timing_id"] and release["exit_status"] == 0
    assert release["request_sha256"] == pilot.run.digest(BOUND / "controller-packet.json")
    replay = pilot.visit(PILOT, BOUND)
    assert json.loads(json.dumps(replay)) == pilot.run.read_json(PILOT / "independent-replay.json")
    assert all(gate["passed"] for gate in replay["gates"].values()) and len(replay["gates"]) == 2
    return qualified, release


def legacy(name):
    path = REPO / "tools/durable-jobs-bench/results/2026-11-23-dense-matched-topology" / name
    data = pilot.canonical("controller-packet.json")
    pin = json.loads(data)["pins"][str(path.relative_to(REPO))]
    assert pilot.run.digest(path) == pin
    return path


def legacy_score():
    qualification = ModuleType("qualification")
    qualification.__file__ = str(legacy("qualification.py"))
    original = sys.modules.get("qualification")
    sys.modules["qualification"] = qualification
    try:
        exec(compile(Path(qualification.__file__).read_bytes(), qualification.__file__, "exec"),
             qualification.__dict__)
        return runpy.run_path(str(legacy("matrix.py")))["score_arm"]
    finally:
        sys.modules["qualification"] = original


def original_peer_gate():
    tree = ast.parse(pilot.canonical("controller.py"))
    functions = [node for node in tree.body if isinstance(node, ast.FunctionDef)
                 and node.name in ("gate", "peer_comparison")]
    assert len(functions) == 2
    scope = dict(ENGINES=ENGINES)
    exec(compile(ast.Module(body=functions, type_ignores=[]), "canonical-peer-comparison.py", "exec"), scope)
    return scope["peer_comparison"]


def remote(label):
    suffix = "off" if label in ("A1", "A2") else "on" if label == "B" else label
    return pilot.run.REMOTE / ID / suffix


def stage_sources(qualified):
    for label in LABELS:
        location = STAGED / label
        if label in ENGINES:
            data = stock_bytes(label)
            location.mkdir(parents=True, exist_ok=False)
            (location / "inventory.json").write_bytes(data)
            continue
        inventory = pilot.staging(label, qualified, location)
        spec = inventory["engines"]["mesh"]
        before, after = map(str, (pilot.remote(label), remote(label)))

        def move(path):
            return after + path[len(before):] if path.startswith(before + "/") else path

        spec["argv"] = [move(arg) for arg in spec["argv"]]
        spec["verified_files"] = {move(path): digest for path, digest in spec["verified_files"].items()}
        inventory_path = location / "inventory.json"
        inventory_path.write_text(json.dumps(inventory, indent=2, sort_keys=True) + "\n")


def stock_bytes(label):
    data = (STOCK / f"{label}.json").read_bytes()
    canonical = pilot.fetch(OLD, f"{pilot.CANONICAL}/peer-inventories/{label}.json")
    recovered = pilot.fetch(OLD, f"{pilot.CANONICAL}/{RC}/evidence/peers/staging/{label}/inventory.json")
    assert data == canonical == recovered
    return data


def manifest():
    qualified, release = released_basis()
    pins = {str(path.relative_to(REPO)): pilot.run.digest(path) for path in (
        HERE / "compare.py", HERE / "preflight.py", HERE / "test_offline.py", legacy("matrix.py"), legacy("qualification.py"),
        PARENT / "pilot.py", PARENT / "controller.py", PILOT / "controller-release.json",
        PILOT / "independent-replay.json", BOUND / "controller-packet.json", PROOF / "verified.json",
        *(STOCK / f"{name}.json" for name in ENGINES),
        *(path for path in sorted(STAGED.rglob("*")) if path.is_file()),
    )}
    source = pilot.canonical("controller.py")
    previous = pilot.fetch(OLD, f"{pilot.CANONICAL}/{RC}/continuation.py")
    return dict(id=ID, grant="TIMING", minutes=35, no_build=True, schedule=SCHEDULE,
                candidate_elf=qualified["elf_sha256"], bridge_elf=pilot.ELF, host=HOST,
                input_sha256=pilot.INPUT, total_slots=64, runners=8, slots_each=8, cpus=[0, 1],
                connection_cap=96, mesh_hot_app_sessions=32, mesh_scheduler_workers=2,
                after=[release["id"], "2026-11-25-public-slow-width32-receipt-recovery-pilot"],
                canonical_peer_revision=OLD, canonical_parser_sha256=sha256(source).hexdigest(),
                canonical_continuation_sha256=sha256(previous).hexdigest(), pins=pins)


def check():
    assert pilot.run.read_json(PACKET) == manifest()
    assert remote("A1") == remote("A2")
    assert {len(order) for order in ORDERS} == {len(LABELS) + 1}
    assert all(set(order) == set(LABELS) | {"A2"} for order in ORDERS)
    for label in LABELS:
        stage = STAGED / label
        inventory = pilot.run.read_json(stage / "inventory.json")
        pilot.run.check_inventory(inventory)
        if label in ENGINES:
            assert (stage / "inventory.json").read_bytes() == stock_bytes(label)
            continue
        subprocess.run(["bash", "-n", stage / "public-native-run"], check=True)
        assert remote(label).parent.parent == pilot.run.REMOTE
        spec = inventory["engines"]["mesh"]
        assert str(remote(label) / "public-native-run") in spec["argv"]
        for path in (stage / "public-native-run", stage / "public_arm.py", pilot.SUITE / "worker.py"):
            assert spec["verified_files"][str(remote(label) / path.name)] == pilot.run.digest(path)
    return pilot.run.read_json(BOUND / "qualified.json")


def score_peer(folder, engine, phase, inventory, score):
    assert pilot.run.read_json(folder / "dispatch.json")["exit_code"] == 0
    row = score(folder, engine, 8, phase, inventory, dict(host=HOST, clock_ticks=100))
    with tarfile.open(folder / "raw.tar.gz") as archive:
        last_effect = max(j["effect_committed_us"] for j in json.load(
            archive.extractfile("adapter/ledger.json"))["jobs"])
        names = set(archive.getnames())
        counters = (json.load(archive.extractfile("adapter/counters.json"))
                    if "adapter/counters.json" in names else None)
    return dict(row, submit_us=row["last_settlement_us"] - row["first_submit_us"],
                drain_us=row["last_settlement_us"] - row["first_commit_us"],
                last_effect_us=last_effect - row["first_commit_us"],
                last_effect_from_submit_us=last_effect - row["first_submit_us"],
                runner_cpu_ms=row["runner_cpu_ms_per_job"] * 1024,
                host_cpu_ms=(row["pinned_user_cpu_ms_per_job"] + row["pinned_kernel_cpu_ms_per_job"]) * 1024,
                peer_sql_calls=counters.get("calls") if isinstance(counters, dict) else None,
                peer_sql_counters=counters)


def score_mesh(folder, label, phase, inventory, qualified, score):
    row = pilot.result(folder, label, phase, inventory, qualified, score)
    row["last_effect_from_submit_us"] = row["last_effect_us"] + row["first_commit_us"] - row["first_submit_us"]
    with tarfile.open(folder / "raw.tar.gz") as archive:
        queue = json.load(archive.extractfile("adapter/queue-policy.json"))
    row["queue_statements"] = queue["queue_statements"]
    return row


def distribution(values):
    return dict(n=len(values), median=median(values), min=min(values), max=max(values))


def summarize(groups):
    measured = {number: {key.removeprefix("measured-"): value for key, value in rows.items()
                         if key.startswith("measured-")} for number, rows in groups.items()}
    comparisons = original_peer_gate()(measured)
    lanes = set(LABELS) | {"A2"}
    ranges = {}
    for lane in sorted(lanes):
        rows = [row[lane] for row in measured.values()]
        counts = {key: distribution([row[key] for row in rows])
                  for key in CLOCKS + RESOURCES if all(row.get(key) is not None for row in rows)}
        ranges[lane] = counts

    pairing = {}
    for lane in sorted(lanes - {"B"}):
        rows = [{clock: row[lane][clock] - row["B"][clock] for clock in CLOCKS}
                for row in measured.values()]
        pairing[lane] = dict(rotations=rows,
                             ranges={clock: distribution([row[clock] for row in rows]) for clock in CLOCKS})
    noise = {clock: distribution([abs(row["A1"][clock] - row["A2"][clock])
                                  for row in measured.values()]) for clock in CLOCKS}
    return dict(groups=groups, matched=comparisons, ranges=ranges, paired_lane_minus_b=pairing,
                same_image_absolute_noise=noise, late_capacity_gates={number: pilot.gate(rows)
                                                                      for number, rows in groups.items()})


def visit(output, qualified, dispatch=None):
    from preflight import verify_receipt
    verify_receipt(pilot.run, STOCK, ENGINES, HOST, pilot.run.digest(PACKET),
                   output / "stock-preflight.json")
    scorer, mesh = legacy_score(), pilot.scorer()
    groups, expected = {}, set()
    for order in SCHEDULE:
        block = output / "blocks" / pilot.run.block_prefix(order)
        if dispatch:
            block.mkdir(parents=True, exist_ok=False)
            pilot.run.write_json(block / "order.json", order)
        assert pilot.run.read_json(block / "order.json") == order
        rows = {}
        for label in order["order"]:
            stage = STAGED / ("A1" if label == "A2" else label)
            inventory = pilot.run.read_json(stage / "inventory.json")
            engine = ENGINES.get(label, "mesh")
            for phase in order["phases"]:
                folder = block / f"{phase}-{label}" / f"{phase}-{engine}"
                if dispatch:
                    dispatch(folder, label, order, phase, engine)
                expected.add(folder / "raw.tar.gz")
                rows[f"{phase}-{label}"] = (score_peer(folder, engine, phase, inventory, scorer)
                                           if label in ENGINES else
                                           score_mesh(folder, label, phase, inventory, qualified, mesh))
        if dispatch:
            pilot.run.write_json(block / "reconciled.json", rows)
        assert pilot.run.read_json(block / "reconciled.json") == json.loads(json.dumps(rows))
        groups[str(order["group"])] = rows
    assert expected == set(output.rglob("raw.tar.gz")) and len(expected) == 70
    assert len({row["raw_sha256"] for group in groups.values() for row in group.values()}) == 70
    return summarize(groups)


def dispatch(controller, output):
    fence = runpy.run_path(str(controller / "remaining-booking-fence.py"))
    run = fence["RUN"]
    assert run.SUITE == controller.parent.parent / "tools/durable-jobs-bench"
    packet, qualified = run.read_json(PACKET), check()
    queue = fence["QUEUE"]
    seal = run.digest(PACKET)
    assert (queue / f"{ID}.json").read_bytes() == PACKET.read_bytes()
    for predecessor in packet["after"]:
        release = run.read_json(queue / f"{predecessor}.released.json")
        assert release["id"] == predecessor and release["released_at"]
    assert (queue / f"{packet['after'][0]}.released.json").read_bytes() == (PILOT / "controller-release.json").read_bytes()

    def execute(_controls, grant):
        def active(remaining):
            assert fence["latest"]()[-1] == ("TIMING", ID)
            assert (queue / f"{ID}.json").read_bytes() == PACKET.read_bytes()
            now = datetime.now(timezone.utc)
            assert datetime.fromisoformat(grant["start"]) <= now
            assert (datetime.fromisoformat(grant["end"]) - now).total_seconds() >= remaining

        def arm(folder, label, order, phase, engine):
            active(125)
            folder.parent.mkdir()
            run.dispatch_arm(folder.parent, remote(label), order, phase, engine)

        with (run.SUITE / ".timing.lock").open("rb") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            active(125)
            from interleave import HOST_IDLE
            run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
            output.mkdir(parents=True, exist_ok=False)
            run.write_json(output / "grant.json", grant)
            for label in LABELS:
                stage = STAGED / label
                record = output / "staging" / label
                record.mkdir(parents=True)
                with (record / "stage.log").open("xb") as log:
                    for item in (*sorted(stage.iterdir()), pilot.SUITE / "worker.py"):
                        active(125)
                        run.stage(item, remote(label), log)
                (record / "inventory.json").write_bytes((stage / "inventory.json").read_bytes())
            from preflight import attest
            active(125)
            with (output / "stock-preflight.json").open("xb") as log:
                attest(run, STOCK, ENGINES, HOST, seal, log)
            run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
            run.write_json(output / "scored-before-release.json", visit(output, qualified, arm))
            active(13)
            run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
        return 0

    with (controller / f"{ID}.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return fence["slot"]("TIMING", ID, seal, packet["minutes"], output, execute)["exit_status"]


def main():
    global pilot
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("seal", "check", "dispatch", "replay"))
    parser.add_argument("--controller", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.command == "dispatch":
        controller = args.controller.resolve(strict=True)
        runpy.run_path(str(PARENT / "controller.py"))["load_controller"](controller)
    import pilot

    if args.command == "seal":
        qualified, _ = released_basis()
        if not STAGED.exists():
            stage_sources(qualified)
        pilot.run.write_json(PACKET, manifest())
        check()
        print(pilot.run.digest(PACKET))
    elif args.command == "check":
        check()
        print(pilot.run.digest(PACKET))
    elif args.command == "dispatch":
        return dispatch(controller, args.output.resolve())
    else:
        qualified = check()
        release = pilot.run.read_json(args.output / "controller-release.json")
        assert release["id"] == ID and release["exit_status"] == 0 and release["request_sha256"] == pilot.run.digest(PACKET)
        result = visit(args.output, qualified)
        assert json.loads(json.dumps(result)) == pilot.run.read_json(args.output / "scored-before-release.json")
        print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
