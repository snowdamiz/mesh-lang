"""Build-free canonical public activation, pilot and contingent stock comparison."""
import argparse
from datetime import datetime, timezone
import fcntl
from hashlib import sha256
import json
from pathlib import Path
import re
import runpy
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
SUITE = HERE.parents[1]
ROOT = SUITE.parents[1]
sys.path[:0] = [str(SUITE), str(HERE.parent / "2026-11-23-dense-matched-topology")]

import run
from interleave import HOST_IDLE
import matrix
from score import HOST, INPUT, score_arm

ID = HERE.name
PUBLIC_ELF = "4924b5880247059032786f43f490cf99bde8a0a69adc790994ee01a51ab91058"
TRANSFER = "2026-11-25-public-dense-3c4032d-transfer-logger-off-recovery-pilot"
TRANSFER_SEAL = "11d5dc6853df8de36843cc50580f2ff28656875d64a6c7404094e7b1d2034dd2"
QUAL = HERE / "qualification"
OUT = HERE / "evidence"
LANES = dict(A1="checked-individual", A2="checked-individual",
             B="bounded-grouped-hot", C="bounded-grouped-cold")
ENGINES = dict(river="go", oban="oban", apalis="rust")
ACTIVATION = ("A1", "C", "B")
PILOT = ("A1", "C", "B", "A2")
PEERS = ("A1", "river", "B", "oban", "apalis", "A2")
MINUTES = dict(activation=10, pilot=14, peers=18)


def trial(group, order, phases):
    return dict(group=group, order=list(order), phases=list(phases), profile="noop")


def reversed_trials(order):
    return [trial(i, labels, ("warmup", "measured")) for i, labels in enumerate((order, order[::-1]), 1)]


TRIALS = dict(activation=[trial(0, ACTIVATION, ("diagnostic",))],
              pilot=reversed_trials(PILOT), peers=reversed_trials(PEERS))


def remote(phase, label):
    lane = LANES.get(label, label)
    return run.REMOTE / ID / phase / lane


def engine(label):
    return ENGINES.get(label, "mesh")


def staged(label):
    return [SUITE / "worker.py"] + (sorted((HERE / "stage").iterdir()) if label in LANES else [])


def inventory(phase, label):
    if label in ENGINES:
        inv = run.read_json(HERE / "peer-inventories" / f"{label}.json")
    else:
        inv = run.read_json(HERE / "reference.inventory.json")
        spec = inv["engines"]["mesh"]
        old = next(str(Path(arg).parent) for arg in spec["argv"] if arg.endswith("/public-native-run"))
        spec["argv"] = [arg.replace(old, str(remote(phase, label)), 1) if arg.startswith(old + "/")
                        else LANES[label] if arg == "checked-individual" else arg for arg in spec["argv"]]
        spec["verified_files"] = {path: digest for path, digest in spec["verified_files"].items()
                                  if not path.startswith(old + "/")}
        for path in staged(label):
            spec["verified_files"][str(remote(phase, label) / path.name)] = run.digest(path)
        spec["evidence_globs"].append("canonical-ready.json")
    run.check_inventory(inv)
    assert inv["cpus"] == [0, 1]
    return inv


def qualify():
    receipt = runpy.run_path(str(QUAL / "native.py"))["replay"](
        QUAL, QUAL / "controller-release.json", QUAL / "raw.tar.gz", git_sources=False)
    assert receipt == run.read_json(QUAL / "verified.json")
    assert receipt["elf_sha256"] == PUBLIC_ELF and len(receipt["proofs"]) == 11
    with tarfile.open(QUAL / "raw.tar.gz") as raw:
        manifest = raw.extractfile("./mesh.json").read()
    build = json.loads(manifest)
    assert build["files"]["mesh"] == PUBLIC_ELF
    assert inventory("activation", "A1")["engines"]["mesh"]["artifact_sha256"] == sha256(manifest).hexdigest()
    definitions = {}
    with tarfile.open(QUAL / "source.tar.gz") as source:
        for file, name in (("schema.sql", "claim_batch_set_v1"), ("grouped_schema.sql", "claim_batch_bounded_v1")):
            sql = source.extractfile(f"source/compiler/mesh-rt/src/jobs/{file}").read().decode()
            definition = re.search(rf"CREATE OR REPLACE FUNCTION mesh_jobs\.{name}\(.*?\$\$;", sql, re.S)[0]
            body = definition.split("AS $$", 1)[1].rsplit("$$;", 1)[0]
            definitions[name] = dict(body_sha256=sha256(body.encode()).hexdigest(), owner="parity_migrator",
                                     configuration=["search_path=pg_catalog, pg_temp", "lock_timeout=100ms"],
                                     security_definer=False, worker=True, producer=False, app=False)
    assert definitions["claim_batch_bounded_v1"]["body_sha256"] == (
        "ecf89d59b8ffcfa3b07aea4b2c1533d329795f366ced63b7e4221a1b40040044")
    return build, definitions


def packet():
    files = [HERE / name for name in ("README.md", "controller.py", "score.py", "witness.py",
                                     "test_witness.py", "reference.inventory.json")]
    for root in ("qualification", "stage", "peer-inventories"):
        files.extend(sorted((HERE / root).iterdir()))
    files.extend(SUITE / name for name in ("run.py", "worker.py", "verify.py", "interleave.py"))
    files.extend(HERE.parent / "2026-11-23-dense-matched-topology" / name
                 for name in ("matrix.py", "qualification.py"))
    seals = {path.relative_to(ROOT).as_posix(): run.digest(path) for path in files}
    return dict(id=ID, no_build=True, cpus=[0, 1], host=HOST, input_sha256=INPUT, public_elf_sha256=PUBLIC_ELF,
                runners=8, slots_each=8, warmed_app_sessions=32, physical_connection_cap=96,
                scheduler_workers_each=2, after=TRANSFER, prior_request_sha256=TRANSFER_SEAL,
                plans={phase: dict(schedule=TRIALS[phase], minutes=MINUTES[phase],
                                   grant="SETUP" if phase == "activation" else "TIMING") for phase in TRIALS},
                dispatch_order=["activation", "pilot", "peers"],
                peer_admission="positive sealed replay of both released mirrored pilot blocks", pins=seals)


def check():
    assert run.read_json(HERE / "controller-packet.json") == packet(), "sealed packet differs"
    subprocess.run(["bash", "-n", HERE / "stage/public-native-run"], check=True)
    for label in ENGINES:
        path = HERE.parent / "2026-11-23-dense-matched-topology/evidence/stock/staging" / f"{label}-8x8/inventory.json"
        assert path.read_bytes() == (HERE / "peer-inventories" / f"{label}.json").read_bytes()
        spec = inventory("peers", label)["engines"][engine(label)]
        assert next(pin for name, pin in spec["verified_files"].items() if name.endswith("/worker.py")) == (
            run.digest(SUITE / "worker.py"))
    assert inventory("pilot", "A1") == inventory("pilot", "A2")
    return qualify()


def arms(block, order):
    for label in order["order"]:
        for phase in order["phases"]:
            yield phase, label, block / f"{phase}-{label}" / f"{phase}-{engine(label)}"


def score(folder, phase, label, action, qualification):
    inv = inventory(action, label)
    if label in LANES:
        return score_arm(folder, phase, LANES[label], inv, *qualification)
    assert run.read_json(folder / "dispatch.json")["exit_code"] == 0
    row = matrix.score_arm(folder, engine(label), 8, phase, inv, dict(host=HOST, clock_ticks=100))
    stop = row["last_settlement_us"]
    return dict(row, submit_us=stop-row["first_submit_us"], drain_us=stop-row["first_commit_us"],
                runner_cpu_ms=row["runner_cpu_ms_per_job"] * 1024,
                host_cpu_ms=(row["pinned_user_cpu_ms_per_job"] + row["pinned_kernel_cpu_ms_per_job"]) * 1024)


def gate(rows):
    a, b, duplicate = (rows[label] for label in ("A1", "B", "A2"))
    measurements = {}
    for key in ("submit_us", "drain_us", "last_effect_us", "terminal_xids",
                "runner_cpu_ms", "pg_cpu_ms", "host_cpu_ms"):
        faster, slower = sorted((a[key], duplicate[key]))
        gain = faster - b[key]
        spread = slower - faster
        if key in ("submit_us", "drain_us", "terminal_xids"):
            passed = gain > max(0.05 * faster, spread)
        elif key == "last_effect_us":
            passed = gain > 0
        else:
            passed = b[key] <= slower
        measurements[key] = dict(gain=gain, spread=spread, passed=passed)
    return dict(passed=all(metric["passed"] for metric in measurements.values()), metrics=measurements)


def peer_comparison(groups):
    report = {}
    for label in ENGINES:
        paired = []
        for rows in groups.values():
            mesh, peer = rows["B"], rows[label]
            gaps = {}
            for key in ("submit_us", "drain_us"):
                spread = abs(rows["A1"][key] - rows["A2"][key])
                margin = max(0.05 * peer[key], spread)
                gaps[key] = dict(peer_minus_mesh=peer[key]-mesh[key], margin=margin)
            paired.append(dict(clocks=gaps, mesh_connections=mesh["peak_connections"],
                               peer_connections=peer["peak_connections"], mesh_pg_ms=mesh["pg_cpu_ms"],
                               mesh_runner_ms=mesh["runner_cpu_ms"], peer_runner_ms=peer["runner_cpu_ms"],
                               mesh_host_ms=mesh["host_cpu_ms"], peer_host_ms=peer["host_cpu_ms"],
                               candidate_resource_bounds=all(m["passed"] for k, m in gate(rows)["metrics"].items()
                                                             if not k.endswith("_us") and k != "terminal_xids")))
        clocks = [item for row in paired for item in row["clocks"].values()]
        qualified = all(row["candidate_resource_bounds"] for row in paired)
        mesh_wins = qualified and all(row["peer_minus_mesh"] > row["margin"] for row in clocks)
        peer_wins = qualified and all(-row["peer_minus_mesh"] > row["margin"] for row in clocks)
        report[label] = dict(paired=paired, outcome="mesh" if mesh_wins else label if peer_wins else "inconclusive")
    return report


def replay(action, qualification):
    evidence = OUT / action
    receipt = run.read_json(evidence / "controller-release.json")
    seal = run.digest(HERE / "controller-packet.json")
    assert receipt["id"] == ID + "-" + action and receipt["request_sha256"] == seal
    assert receipt["exit_status"] == 0, f"{action} has no successful release"
    groups = {}
    expected = set()
    for order in TRIALS[action]:
        block = evidence / "blocks" / run.block_prefix(order)
        assert run.read_json(block / "order.json") == order
        rows = {}
        for phase, label, folder in arms(block, order):
            expected.add(folder / "raw.tar.gz")
            rows[f"{phase}-{label}"] = score(folder, phase, label, action, qualification)
        assert json.loads(json.dumps(rows)) == run.read_json(block / "reconciled.json")
        groups[str(order["group"])] = rows
    assert expected == set(evidence.rglob("raw.tar.gz"))
    measured = {group: {label: rows["measured-" + label] for label in order["order"]}
                for (group, rows), order in zip(groups.items(), TRIALS[action], strict=True)} if action != "activation" else {}
    if action == "pilot":
        decision = {group: gate(rows) for group, rows in measured.items()}
        return dict(groups=groups, gate=decision, advance_to_peers=all(row["passed"] for row in decision.values()))
    if action == "peers":
        return dict(groups=groups, matched=peer_comparison(measured))
    return dict(groups=groups, activated=True)


def active(action, queue, seal):
    ident = ID + "-" + action
    source = HERE / "controller-packet.json"
    assert (queue / f"{ident}.json").read_bytes() == source.read_bytes() and run.digest(source) == seal
    last = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+) (\S+?)(?:,? |$)([^\s,]*)",
                      (queue / "STATUS.md").read_text(), re.M)[-1]
    kind, name, start, end = last
    assert (kind, name) == ("SETUP" if action == "activation" else "TIMING", ident)
    assert datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
    return dict(id=ident, start=start, end=end, request_sha256=seal)


def dispatch(action, seal, controller_root, qualification):
    queue = controller_root / "target/linode-jobs-stepchange/requests"
    prior = run.read_json(queue / f"{TRANSFER}.released.json")
    assert prior["id"] == TRANSFER and type(prior["exit_status"]) is int
    assert prior["request_sha256"] == TRANSFER_SEAL
    receipt = active(action, queue, seal)
    if action == "pilot":
        assert replay("activation", qualification)["activated"]
    if action == "peers":
        assert replay("pilot", qualification)["advance_to_peers"], "pilot peer gate closed"
    access = controller_root / "target/linode-jobs-access"
    assert (access / "known_hosts").is_file() and (access / "id_ed25519").is_file()
    run.SSH = [str(item).replace(str(run.ACCESS), str(access)) for item in run.SSH]
    evidence = OUT / action
    with (controller_root / "tools/durable-jobs-bench/.timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        assert active(action, queue, seal) == receipt
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
        evidence.mkdir(parents=True, exist_ok=False)
        run.write_json(evidence / "grant.json", receipt)
        for label in dict.fromkeys(label for trial in TRIALS[action] for label in trial["order"]):
            if label == "A2":
                continue
            directory = evidence / "staging" / label
            directory.mkdir(parents=True)
            inv = directory / "inventory.json"
            if label in ENGINES:
                inv.write_bytes((HERE / "peer-inventories" / f"{label}.json").read_bytes())
            else:
                run.write_json(inv, inventory(action, label))
            with (directory / "stage.log").open("xb") as log:
                for path in (*staged(label), inv):
                    assert active(action, queue, seal) == receipt
                    run.stage(path, remote(action, label), log)
        for order in TRIALS[action]:
            block = evidence / "blocks" / run.block_prefix(order)
            block.mkdir(parents=True)
            run.write_json(block / "order.json", order)
            rows = {}
            for phase, label, folder in arms(block, order):
                assert active(action, queue, seal) == receipt
                assert (datetime.fromisoformat(receipt["end"]) - datetime.now(timezone.utc)).total_seconds() >= 125
                folder.parent.mkdir()
                run.dispatch_arm(folder.parent, remote(action, label), order, phase, engine(label))
                rows[f"{phase}-{label}"] = score(folder, phase, label, action, qualification)
            run.write_json(block / "reconciled.json", rows)
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("seal", "check", "dispatch", "replay"))
    parser.add_argument("action", choices=tuple(TRIALS), nargs="?")
    parser.add_argument("seal", nargs="?")
    parser.add_argument("--controller-root", type=Path)
    args = parser.parse_args()
    if args.command == "seal":
        run.write_json(HERE / "controller-packet.json", packet())
    elif args.command == "check":
        check()
        print(run.digest(HERE / "controller-packet.json"))
    elif args.command == "replay":
        print(json.dumps(replay(args.action, check()), indent=2, sort_keys=True))
    else:
        dispatch(args.action, args.seal, args.controller_root.resolve(strict=True), check())


if __name__ == "__main__":
    main()
