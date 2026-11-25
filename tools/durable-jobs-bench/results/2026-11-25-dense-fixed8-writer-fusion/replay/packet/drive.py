"""Prepare and replay the scoped writer fusion run. Only the controller dispatches."""
from copy import deepcopy
from datetime import datetime, timezone
import fcntl
import hashlib
import json
from pathlib import Path
import re
import runpy
import sys
import tarfile
import time

from fusion_score import gate, score

HERE = Path(__file__).resolve().parent
WORK = HERE.parent
ROOT = WORK.parents[1]
SUITE = ROOT / "tools/durable-jobs-bench"
WINNER = SUITE / "results/2026-11-25-dense-fixed8-logical-factorial"
PIN = WINNER / "controller-packet-retry.json"
sys.path.insert(0, str(WINNER))
base = runpy.run_path(str(WINNER / "controller.py"))
base.update({key: value for key, value in runpy.run_path(str(WINNER / "score.py")).items()
             if key in {"measure", "CONTRACTION"}})
run, grouped = base["run"], base["grouped"]
QUEUE = base["QUEUE"]
OUT = WORK / "evidence"
ARCHIVE = HERE / "source.tar.gz"
PACKET = HERE / "controller-packet.json"
SQL = "d388baa3b67c07df1a2498cd1fdc60bf7d0ce11f4b42d950f7b0a64a551c0f45"
NAME = "dense-fixed8-writer-fusion"
SETUP, SMOKE, PILOT, PEERS_ID = (f"{NAME}-{name}" for name in ("setup", "smoke", "pilots", "peers"))
FILES = tuple(HERE / name for name in ("launch.sh", "render.py", "factorial_arm.py",
                                      "contract.py", "stock.sql", "candidate.sql"))
MODES = dict(bridge="fixed8ms", **{"off-a": "fixed8ms", "off-b": "fixed8ms"},
             positive="fused_fixed8ms", fused="fused_fixed8ms")
NATIVE = Path("/var/lib/durable-jobs-bench/native")
REPLAY = Path(str(grouped.REPLAY))
FAULTS = ("sql-cut", "confirm-commit", "malformed-grant", "lost-commit",
          "stop-at-commit", "pending-ordinary", "teardown-release", "teardown-kill")
block = base["block"]
ORDER = ("off-a", "fused", "bridge", "off-b")
PRIMARY = {SMOKE: [block(0, ("positive",), ("diagnostic",))],
           PILOT: [block(1, ORDER), block(2, ORDER[::-1])],
           PEERS_ID: [dict(trial, order=["fused" if lane == "combined" else lane
                                        for lane in trial["order"]]) for trial in base["PEER_TRIALS"]]}


def remote(lane):
    return run.REMOTE / f"{NAME}-{lane}"


def check():
    spec = run.read_json(PACKET)
    assert spec["schedule"] == json.loads(json.dumps(PRIMARY))
    for path, sha in spec["local_sha256"].items():
        assert run.digest(ROOT / path) == sha, path
    assert run.digest(HERE / "candidate.sql") == SQL
    assert run.read_json(PIN)["candidate_sha256"] == SQL
    assert run.digest(base["SOURCE"]) == base["SOURCE_SHA"]
    return spec


def attempt():
    return "trusted-jobs-product-" + run.digest(ARCHIVE)[:12]


def active(key, kind):
    assert (QUEUE / f"{key}.json").read_bytes() == PACKET.read_bytes(), "registered packet mismatch"
    text = (QUEUE / "STATUS.md").read_text()
    mode, name, start, end = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD) (\S+)(?: ([^\s,]+))?(?: ([^\s,]+))?",
                                         text, re.M)[-1]
    assert (mode, name) == (kind, key), "waiting for the exact controller booking"
    assert datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
    return dict(id=key, start=start, end=end, request_sha256=run.digest(PACKET))


def spec():
    receipt = active(SETUP, "SETUP")
    with tarfile.open(ARCHIVE) as data:
        script = data.extractfile("source/" + str(REPLAY / "native-setup.sh")).read().decode()
    return dict(receipt=receipt, attempt=attempt(), script=script, stdin_archive=str(ARCHIVE),
                arguments=[attempt(), grouped.BASE, run.digest(ARCHIVE)],
                remote_evidence=str(NATIVE / "runs" / attempt()))


def bind(raw_path, release_path):
    release = run.read_json(release_path)
    assert release["id"] == SETUP and release["exit_status"] == 0 and release["released_at"]
    with tarfile.open(raw_path) as raw, tarfile.open(ARCHIVE) as source:
        get = grouped.public.archive_files(raw)
        read = grouped.public.archive_files(source, "source/")
        manifest = get("mesh.json")
        build, witness = json.loads(manifest), json.loads(get("witness.json"))
        for artifact in (build, witness):
            assert (artifact["system"], artifact["architecture"], artifact["version"],
                    artifact["source_sha256"], artifact["compiler_sha256"], artifact["execution_contract"]) == (
                        "Linux", "x86_64", grouped.BASE, run.digest(ARCHIVE),
                        grouped.public.COMPILER_SHA, "ordinary-trusted-shared-process-v2")
        assert witness["runtime_sha256"] == build["runtime_sha256"]
        for name in ("main.mpl", "writes.mpl", "worker_identity.c", "worker_identity.mpl"):
            assert hashlib.sha256(read(str(REPLAY.parent / "fixture" / name))).hexdigest() == build["files"][name]
        assert all(pin in get("toolchain.txt").decode() for pin in (*base["logical"].HOST, "17.11", "21.1.8"))
        original = {name: json.loads(get(f"semantic-root/semantic/{name}/result.json"))
                    for name in grouped.PROOFS}
        grouped.validate_proofs(original, witness["files"]["mesh"])
        proofs = {name: json.loads(get(f"semantic-root/fused/{name}/result.json")) for name in FAULTS}
        assert proofs["sql-cut"]["contention_rolled_back"] and proofs["sql-cut"]["contraction_preserved"]
        assert proofs["pending-ordinary"]["overlap"]
        for name in ("confirm-commit", "lost-commit", "stop-at-commit"):
            assert proofs[name]["wire"]["command"] == "COMMIT" and proofs[name]["wire"]["idle"]
        assert proofs["lost-commit"]["wire"]["receipt_dropped"]
        assert proofs["malformed-grant"]["wire"]["corrupt_grant"]
        assert proofs["teardown-release"]["done"]["state"] == "completed"
        assert proofs["teardown-kill"]["done"]["state"] == "executing"
    return dict(build=build, manifest_sha256=hashlib.sha256(manifest).hexdigest(), attempt=attempt(),
                source_sha256=run.digest(ARCHIVE), setup_raw_sha256=run.digest(raw_path),
                release_sha256=run.digest(release_path), faults=list(FAULTS))


def config(binding):
    cfg = base["config"]()
    original = cfg["lanes"]["combined"]["inventory"]
    old_attempt = "trusted-jobs-product-" + base["SOURCE_SHA"][:12]
    old_stage = base["remote"]("combined")
    for lane, policy in MODES.items():
        selected = deepcopy(original)
        mesh = selected["engines"]["mesh"]
        pins = mesh["verified_files"]
        if lane != "bridge":
            build = binding["build"]
            mesh["artifact_sha256"] = binding["manifest_sha256"]
            spec_files = dict(build["files"], **{"mesh.json": binding["manifest_sha256"]})
            for path in list(pins):
                if f"/builds/{old_attempt}/" in path:
                    del pins[path]
                    pins[path.replace(old_attempt, binding["attempt"])] = spec_files[Path(path).name]
                elif f"/source-{old_attempt}/" in path:
                    del pins[path]
                    new = path.replace(old_attempt, binding["attempt"])
                    relative = new.split("/source/", 1)[1]
                    with tarfile.open(ARCHIVE) as source:
                        pins[new] = hashlib.sha256(source.extractfile("source/" + relative).read()).hexdigest()
            mesh["argv"] = [arg.replace(old_attempt, binding["attempt"]).replace(
                base["ELF"], build["files"]["mesh"]).replace(
                cfg["product"]["manifest_sha256"], binding["manifest_sha256"]) for arg in mesh["argv"]]
        mesh["argv"][1] = f"JOBS_GROUPED_WRITER={policy}"
        assert mesh["argv"][-2:] == ["combined", "mesh"]
        mesh["argv"][-2] = lane
        mesh["argv"] = [arg.replace(str(old_stage), str(remote(lane))) for arg in mesh["argv"]]
        for path in [path for path in pins if Path(path).parent == old_stage]:
            del pins[path]
        pins.update({str(remote(lane) / path.name): run.digest(path) for path in (*FILES, SUITE / "worker.py")})
        run.check_inventory(selected)
        cfg["lanes"][lane] = dict(engine="mesh", runners=2, inventory=selected)
    for lane in base["PEERS"]:
        item = cfg["lanes"][lane]
        pins = item["inventory"]["engines"][item["engine"]]["verified_files"]
        old = str(base["remote"](lane) / "worker.py")
        pins[str(remote(lane) / "worker.py")] = pins.pop(old)
    return cfg


def arm_score(path, phase, lane, cfg, binding):
    if lane in MODES and lane != "bridge":
        cfg = dict(cfg, product=binding)
    return score(path, phase, lane, cfg, base)


def clean_release(key):
    ends = PRIMARY[key]
    receipt = run.read_json(base["group_release"](key, ends[0]["group"], ends[-1]["group"]))
    assert receipt["id"] == key and receipt["exit_status"] == 0 and receipt["released_at"]
    assert receipt["receipt"] == str(OUT)


def replay(key, cfg, binding):
    groups, failures = {}, {}
    for trial in PRIMARY[key]:
        for path in sorted((OUT / "blocks").glob(f"{run.block_prefix(trial)}.*")):
            try:
                assert run.read_json(path / "order.json") == trial
                phases = {}
                attempts = set()
                for phase, lane, _, folder in grouped.matrix.paths(path, cfg, trial):
                    request = run.read_json(folder / "request.json")["attempt"]
                    assert request not in attempts
                    attempts.add(request)
                    phases.setdefault(phase, {})[lane] = arm_score(folder, phase, lane, cfg, binding)
                assert trial["group"] not in groups, "extra complete block"
                groups[trial["group"]] = phases
            except (OSError, ValueError, KeyError, AssertionError, RuntimeError, tarfile.TarError) as error:
                failures[path.name] = f"{type(error).__name__}: {error}"
    result = dict(groups=groups, failures=failures, complete=set(groups) == {b["group"] for b in PRIMARY[key]},
                  archives={str(p.relative_to(OUT)): run.digest(p) for p in sorted((OUT / "blocks").rglob("raw.tar.gz"))})
    if key == PILOT:
        result["advances"] = result["complete"] and not failures and all(gate(block["measured"])["passed"]
                                                                         for block in groups.values())
        result["gates"] = {i: gate(w["measured"]) for i, w in groups.items()}
    if key == PEERS_ID and result["complete"] and not failures:
        result["pairs"] = base["peer_pairs"](
            [dict(measured=dict(row["measured"], combined=row["measured"]["fused"]))
             for _, row in sorted(groups.items())], base["PEERS"])
    return result


def dispatch(key, cfg, binding):
    if key in (PILOT, PEERS_ID):
        clean_release(SMOKE)
        smoke = replay(SMOKE, cfg, binding)
        assert smoke["complete"] and not smoke["failures"]
    if key == PEERS_ID:
        clean_release(PILOT)
        assert replay(PILOT, cfg, binding)["advances"]
    from interleave import HOST_IDLE

    receipt = active(key, "TIMING")
    OUT.mkdir(exist_ok=True)
    with (SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        active(key, "TIMING")
        run.ssh(["bash", "-c", HOST_IDLE], timeout=12).check_returncode()
        nonce = time.time_ns()
        run.write_json(OUT / f"grant-{nonce}.json", receipt)
        for lane in sorted({lane for trial in PRIMARY[key] for lane in trial["order"]}):
            folder = OUT / "staging" / f"{key}.{nonce}" / lane
            folder.mkdir(parents=True)
            inventory = folder / "inventory.json"
            run.write_json(inventory, cfg["lanes"][lane]["inventory"])
            with (folder / "stage.log").open("xb") as log:
                for source in (*(FILES if lane in MODES else ()), SUITE / "worker.py", inventory):
                    active(key, "TIMING")
                    run.stage(source, remote(lane), log)
        for trial in PRIMARY[key]:
            path = OUT / "blocks" / f"{run.block_prefix(trial)}.{time.time_ns()}"
            path.mkdir(parents=True)
            run.write_json(path / "order.json", trial)
            phases = {}
            for phase, lane, item, folder in grouped.matrix.paths(path, cfg, trial):
                end = datetime.fromisoformat(active(key, "TIMING")["end"])
                assert (end - datetime.now(timezone.utc)).total_seconds() >= 125
                folder.parent.mkdir()
                run.dispatch_arm(folder.parent, remote(lane), trial, phase, item["engine"])
                phases.setdefault(phase, {})[lane] = arm_score(folder, phase, lane, cfg, binding)
            run.write_json(path / "reconciled.json", phases)
    return 0


def main(args):
    mode, *rest = args
    check()
    if mode == "check":
        return 0
    if mode == "setup-spec":
        print(json.dumps(spec(), indent=2))
        return 0
    if mode == "bind":
        print(json.dumps(bind(*map(Path, rest)), indent=2))
        return 0
    key, raw, release = rest
    binding = bind(Path(raw), Path(release))
    cfg = config(binding)
    if mode == "inventory":
        print(json.dumps(cfg, indent=2))
    elif mode == "replay":
        print(json.dumps(replay(key, cfg, binding), indent=2))
    else:
        assert mode == "dispatch"
        return dispatch(key, cfg, binding)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
