#!/usr/bin/env python3
"""Replay the hot readiness transfer. SETUP and timing are sole-controller commands."""
from datetime import datetime, timezone
import fcntl
from io import BytesIO
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
REF = HERE.parent / "2026-11-24-dense-individual-hot-pool"
sys.path.insert(0, str(REF))
import importlib.util

spec = importlib.util.spec_from_file_location("previous_dense", REF / "controller.py")
previous = importlib.util.module_from_spec(spec)
spec.loader.exec_module(previous)

dense, matrix, run, require = previous.dense, previous.matrix, previous.run, previous.require
ID = HERE.name + "-logger-off"
STAGE = dense.BASE / "dense-hot-readiness-1e70f18d-logger-off"
REMOTE_SETUP = dense.BASE / "runs" / STAGE.name
OUT = HERE / "evidence"
SOURCE = previous.SOURCE / "source.tar.gz"
DELTA = HERE / "runtime.patch"
PEERS = tuple(dense.PEERS)
CONTROLS = ("hot-sync-a", "hot-sync-b")
MESH = (*CONTROLS, "hot-ready")
CLOCKS = ("submit_us", "drain_us", "middle_effect_us")
CASES = ("hot-sync", "hot-ready-stale", "hot-ready-lost-receipt", "hot-ready-cancel", "hot-ready-kill")
FILES = {name: HERE / name for name in ("adapter.py", "native-run", "native-proof.py", "runtime.patch")}
FILES.update({"base_adapter.py": REF / "adapter.py", "identity.py": REF / "identity.py", "source.tar.gz": SOURCE})
LOGGER_OFF = REF / "logger-off-retry/main.mpl"
LOGGER_OFF_SHA = "97b8e75f8568191ac2b3a3160ce9c40d0eebc822ba2ec4661259510734816661"
SHARED_LOGGER = LOGGER_OFF.with_name("logger.py")
SHARED_LOGGER_SHA = "085c89c3cbdd25e44d833318d49932748774e498fff90975b08227b048459046"
FILES.update({"main-logger-off.mpl": LOGGER_OFF, "logger.py": SHARED_LOGGER})
sys.path.insert(0, str(SHARED_LOGGER.parent))
from logger import verify_dense_logs
from logging_contract import verify_runner_log

original_stages, original_mesh = previous.stages, previous.mesh

previous.STAGE = previous.PROOF_STAGE = STAGE
previous.ARTIFACT = STAGE / "artifacts/jobs"
previous.adapter_files = lambda: {name: run.digest(path) for name, path in FILES.items() if name != "source.tar.gz"}


def packet(phase):
    path = HERE / "logger-off" / f"{phase}.json"
    data = run.read_json(path)
    require(data["id"] == f"{ID}-{phase}" and data["source_sha"] == previous.REV, "wrong experiment")
    for name, pin in data["local_sha256"].items():
        require(run.digest(dense.ROOT / name) == pin, f"sealed input differs: {name}")
    return path, data


def schedule(phase):
    forward = ["hot-sync-a", "hot-ready", "hot-sync-b"]
    if phase == "peers":
        river, oban, apalis = PEERS
        forward = ["hot-sync-a", river, oban, "hot-ready", apalis, "hot-sync-b"]
    return [dict(group=index, order=order, profile="noop", phases=["warmup", "measured"])
            for index, order in enumerate((forward, forward[::-1]), 1)]


def config(binding, phase):
    cfg = previous.config(binding)
    cfg["description"] = f"Public individual-terminal hot readiness {phase}"
    cfg["lanes"] = {label: (cfg["lanes"][label] if label in PEERS else dict(engine="mesh", runners=2))
                    for label in (*MESH, *(PEERS if phase == "peers" else ()))}
    return cfg


def stages(cfg, label, remote):
    files, inv = original_stages(cfg, label, remote)
    if label in MESH:
        mode = "hot-ready" if label == "hot-ready" else "hot-sync"
        inv["engines"]["mesh"]["argv"][1] = f"CHECKOUT_MODE={mode}"
        run.check_inventory(inv)
    return files, inv


def scored_mesh(get, log, phase, _hot, binding, executable):
    receipt = get("readiness-mode")
    require(receipt == dict(mode=receipt["mode"], trace=False, pool_cap=16)
            and receipt["mode"] in {"hot-sync", "hot-ready"}
            and all("postgres socket wait" not in log(f"runner-{i}.log") for i in range(2)),
            "timed arm used the diagnostic observer or wrong pool")
    verify_dense_logs(get, log)
    return dict(original_mesh(get, log, phase, True, binding, executable), mode=receipt["mode"])


def score_arm(folder, phase, label, cfg, remote):
    score = previous.score_arm(folder, phase, label, cfg, remote)
    if label in MESH:
        expected = "hot-ready" if label == "hot-ready" else "hot-sync"
        require(score["mode"] == expected, "recorded mode differs from the staged invocation")
    return dict(score, final_effect_us=score["last_effect_us"] - score["first_commit_us"])


previous.stages, previous.mesh = stages, scored_mesh


def payload():
    buffer = BytesIO()
    with tarfile.open(fileobj=buffer, mode="w") as archive:
        files = {name: path.read_bytes() for name, path in FILES.items()}
        files["installed.sha256"] = "".join(f"{dense.digest(data)}  {name}\n" for name, data in files.items()).encode()
        for name, data in files.items():
            entry = tarfile.TarInfo(name)
            entry.size = len(data)
            entry.mode = 0o644
            archive.addfile(entry, BytesIO(data))
    return buffer.getvalue()


def setup_receipt():
    folder = OUT / "setup"
    path, data = packet("setup")
    grant = run.read_json(folder / "grant.json")
    require(run.read_json(folder / "status.json") == {"exit_code": 0}, "SETUP command failed")
    require(grant["id"] == data["id"] and grant["mode"] == "SETUP"
            and grant["request_sha256"] == run.digest(path), "missing sealed SETUP grant")
    raw = folder / "raw.tar.gz"
    with tarfile.open(raw) as archive:
        get = dense.archive_files(archive)
        require(get("native.exit").strip() == b"0" and get("SETUP-PASS") == b"",
                "native setup or proof failed")
        installed = {name: pin for pin, name in (line.split() for line in get("installed.sha256").decode().splitlines())}
        require(installed == {name: run.digest(path) for name, path in FILES.items()}, "installed bytes differ")
        manifest_bytes = get("mesh.json")
        manifest = json.loads(manifest_bytes)
        require(manifest["version"] == previous.REV and manifest["source_sha256"] == run.digest(DELTA)
                and manifest["architecture"] == "x86_64" and manifest["system"] == "Linux"
                and manifest["compiler_sha256"] == dense.COMPILER_SHA
                and b"ELF64" in get("mesh-elf.txt"), "candidate manifest differs")
        with tarfile.open(SOURCE) as source:
            fixture = dense.archive_files(source, "overlay/fixture/")
            require(manifest["files"]["main.mpl"] == LOGGER_OFF_SHA, "wrong canonical logger-off fixture")
            for name in ("writes.mpl", "worker_identity.mpl", "worker_identity.c"):
                require(manifest["files"][name] == dense.digest(fixture(name)), f"frozen handler differs: {name}")
        require(get("proof/cases/PASS") == b"", "compiled proof did not finish")
        cases = {name: json.loads(get(f"proof/cases/{name}/result.json")) for name in CASES}
        require(not cases["hot-ready-cancel"]["killed"] and cases["hot-ready-kill"]["killed"]
                and cases["hot-ready-lost-receipt"]["app_insert_calls"] == 3
                and cases["hot-ready-lost-receipt"]["wire"]["receipt_dropped"]
                and len(set(cases["hot-ready-stale"]["xids"])) == 9, "proof witness differs")
        for name in CASES:
            verify_runner_log(get(f"proof/cases/{name}/runner.log").decode(),
                              trace=name != "hot-sync", killed=name == "hot-ready-kill",
                              lost_receipt=name == "hot-ready-lost-receipt")
        for name in CASES[1:]:
            require(b"postgres socket wait read" in get(f"proof/cases/{name}/runner.log"), "missing TLS wait")
        binding = dict(manifest=manifest, manifest_sha256=dense.digest(manifest_bytes),
                       proof_raw_sha256=run.digest(raw))
        def bridge_log(name):
            return get(f"cold-ready/{name}").decode()

        def bridge_json(name):
            return json.loads(bridge_log(f"{name}.json"))

        verify_dense_logs(bridge_json, bridge_log)
        bridge = original_mesh(bridge_json, bridge_log, "diagnostic", False, binding, previous.ARTIFACT / "mesh")
        require(json.loads(get("cold-ready/readiness-mode.json")) == dict(mode="cold-ready", trace=True, pool_cap=16)
                and all(b"postgres socket wait read" in get(f"cold-ready/runner-{i}.log") for i in range(2)),
                "cold bridge did not suspend both runners")
    return dict(binding=binding, proof=cases, bridge=bridge)


def setup():
    path, data = packet("setup")
    with (matrix.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        receipt = previous.grant(path)
        remaining = (datetime.fromisoformat(receipt["end"]) - datetime.now(timezone.utc)).total_seconds() - 120
        require(remaining >= 600, "insufficient SETUP time")
        folder = OUT / "setup"
        folder.mkdir(parents=True, exist_ok=False)
        run.write_json(folder / "grant.json", receipt)
        stage = payload()
        require(dense.digest(stage) == data["payload_sha256"], "SETUP payload differs")
        code = None
        try:
            with (folder / "setup.log").open("xb") as log:
                code = run.ssh(["bash", "-c", (HERE / "setup.sh").read_text(), "--",
                                *config({}, "pilot")["host"], data["payload_sha256"], run.digest(DELTA)],
                               input=stage, stdout=log, stderr=log, timeout=remaining).returncode
        finally:
            run.write_json(folder / "status.json", {"exit_code": code})
            with (folder / "raw.tar.gz").open("xb") as raw, (folder / "export.log").open("xb") as log:
                exported = run.ssh(["bash", "-c", previous.HOST_IDLE + '\nexec tar -czf - -C "$1" .',
                                    "--", REMOTE_SETUP], stdout=raw, stderr=log, timeout=55)
        require(code == 0 and exported.returncode == 0, "SETUP failed; reconcile before retry")
        run.write_json(folder / "verified.json", setup_receipt())


def contrast(scores):
    a, b, candidate = [scores[f"measured-{label}"] for label in MESH]
    clocks = {}
    for key in (*CLOCKS, "final_effect_us"):
        faster = min(a[key], b[key])
        saving = faster - candidate[key]
        threshold = max(abs(a[key] - b[key]), .05 * faster) if key in CLOCKS else 0
        clocks[key] = dict(stock_a=a[key], stock_b=b[key], candidate=candidate[key], saving=saving,
                           required_saving=threshold, passes=saving > threshold)
    resources = {}
    for key, limit in (("runner_cpu_ms_per_job", 1.05), ("host_cpu_ms_per_job", 1.05),
                       ("peak_runner_rss_bytes", 1.10)):
        def value(row):
            return (row["pinned_user_cpu_ms_per_job"] + row["pinned_kernel_cpu_ms_per_job"]
                    if key == "host_cpu_ms_per_job" else row[key])
        cap = max(value(a), value(b)) * limit
        resources[key] = dict(stock_a=value(a), stock_b=value(b), candidate=value(candidate), cap=cap,
                              passes=value(candidate) <= cap)
    wins = {peer: {key: candidate[key] < scores[f"measured-{peer}"][key]
                   for key in (*CLOCKS, "final_effect_us")} for peer in PEERS if f"measured-{peer}" in scores}
    return dict(clocks=clocks, resources=resources, peer_wins=wins,
                passes=all(row["passes"] for row in (*clocks.values(), *resources.values())))


def replay_block(path, block, cfg, remotes):
    require(run.read_json(path / "order.json") == block, "rotation order differs")
    return {f"{phase}-{label}": score_arm(folder, phase, label, cfg, run.REMOTE / remotes[label])
            for phase, label, _, folder in matrix.paths(path, cfg, block)}


def summarize(directory, cfg, blocks, pins):
    manifest = run.read_json(directory / "series.json")
    require(all(manifest[k] == v for k, v in dict(config=cfg, schedule=blocks, pins=pins).items()),
            "series parameters drifted")
    failures, selected, missing = {}, {}, []
    for block in blocks:
        group = block["group"]
        found = matrix.pick(directory, block, cfg, manifest["remotes"], failures)
        if found is None:
            missing.append(group)
        else:
            name, scores = found
            selected[group] = dict(attempt=name, scores=scores, contrast=contrast(scores))
    raw = {str(path.relative_to(directory)): run.digest(path) for path in directory.rglob("raw.tar.gz")}
    return dict(blocks=selected, failures=failures, missing=missing, raw=raw, advances=not missing and
                all(row["contrast"]["passes"] for row in selected.values()))


matrix.stages, matrix.replay_block, matrix.summarize = stages, replay_block, summarize
matrix.ERRORS += (AssertionError,)


def released(folder, ident):
    release = folder / "controller-release.json"
    data = run.read_json(release)
    require(data["id"] == ident and data["exit_status"] == 0, "successful native controller release required")
    return run.digest(release)


def series(phase):
    qualification = setup_receipt()
    require(run.read_json(OUT / "setup/verified.json") == qualification, "SETUP proof replay differs")
    path, data = packet(phase)
    blocks = schedule(phase)
    require(data["schedule"] == blocks, "sealed order differs")
    binding = qualification["binding"]
    pins = dict(data["local_sha256"], request=run.digest(path), setup=binding["proof_raw_sha256"],
                setup_release=released(OUT / "setup", f"{ID}-setup"))
    folder = OUT / phase
    return path, folder, config(binding, phase), blocks, pins


def timing(phase, dispatch):
    path, directory, cfg, blocks, pins = series(phase)
    if phase == "peers":
        pilot_release = released(OUT / "pilot", f"{ID}-pilot")
        pilot = summarize(*series("pilot")[1:])
        require(pilot["advances"], "both reversed pilots must pass before any peer rotation")
        pins["pilot_release"] = pilot_release
        pins["pilot_raw"] = pilot["raw"]
    if dispatch:
        matrix.grant = lambda _: previous.grant(path)
        matrix.SERIES["equal"] = f"{ID}-{phase}"
        matrix.dispatch(directory, cfg, "equal", blocks, pins, [1, 2])
    result = summarize(directory, cfg, blocks, pins)
    print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))
    return int(bool(result["missing"]))


def check():
    require(run.digest(LOGGER_OFF) == LOGGER_OFF_SHA and run.digest(SHARED_LOGGER) == SHARED_LOGGER_SHA,
            "shared logger-off source or verifier differs")
    with tarfile.open(SOURCE) as archive:
        original = dense.archive_files(archive)("overlay/fixture/main.mpl")
    flag = br',\"event_log\":true'
    require(original.count(flag) == 1 and original.replace(flag, b"") == LOGGER_OFF.read_bytes(),
            "fixture changes beyond the hardcoded logger flag")
    for phase in ("setup", "pilot", "peers"):
        _, data = packet(phase)
        if phase == "setup":
            require(dense.digest(payload()) == data["payload_sha256"], "SETUP payload differs")
        else:
            require(data["schedule"] == schedule(phase), "rotation order differs")
    expected = {"manifest": {"files": {"mesh": "0" * 64}}, "manifest_sha256": "1" * 64}
    cfg = config(expected, "peers")
    remote = run.REMOTE / "readiness-check"
    synchronized = stages(cfg, CONTROLS[0], remote)
    require(synchronized == stages(cfg, CONTROLS[1], remote), "hot sync controls differ")
    synchronized[1]["engines"]["mesh"]["argv"][1] = "CHECKOUT_MODE=hot-ready"
    require(synchronized == stages(cfg, "hot-ready", remote), "more than readiness switch changed")
    for peer in PEERS:
        require(stages(cfg, peer, remote) == dense.stages(dense.configurations({}), peer, remote), "peer inventory drift")
    for name in ("setup.sh", "native-run"):
        subprocess.run(["bash", "-n", HERE / name], check=True)
    for name in ("adapter.py", "native-proof.py"):
        compile((HERE / name).read_bytes(), name, "exec")
    with tempfile.TemporaryDirectory(dir=HERE) as tmp:
        root = Path(tmp)
        with tarfile.open(SOURCE) as archive:
            archive.extractall(root, filter="data")
        with tarfile.open(previous.HARNESS_ARCHIVE) as archive:
            archive.extractall(root, members=(m for m in archive if m.name.startswith("ordinary-v2/")),
                               filter="data")
        subprocess.run(["git", "apply", "--no-index", "--check", DELTA], cwd=root / "source", check=True)
        env = dict(os.environ, PRODUCT_SOURCE=str(root / "source"),
                   PRODUCT_FROZEN_HARNESS=str(root / "ordinary-v2"), CHECKOUT_MODE="hot-ready",
                   DURABLE_JOBS_PHASE="measured",
                   PYTHONPATH=os.pathsep.join(map(str, (REF, SHARED_LOGGER.parent))))
        subprocess.run([sys.executable, "-B", "-c",
                        "import sys, adapter as base_adapter; "
                        "sys.modules['base_adapter'] = base_adapter; "
                        "sys.path.insert(0, sys.argv[1]); sys.modules.pop('adapter'); "
                        "import adapter; adapter.base.install()", str(HERE)],
                       cwd=root, env=env, check=True)
    print("Verified exact payload, duplicate controls, single readiness switch, reversed orders and all three peer inventories.")


if __name__ == "__main__":
    match sys.argv[1:]:
        case ["check"]:
            check()
        case ["setup"]:
            setup()
        case ["replay", "setup"]:
            print(json.dumps(setup_receipt(), indent=2, sort_keys=True))
        case [action, phase] if action in {"timing", "replay"} and phase in {"pilot", "peers"}:
            sys.exit(timing(phase, dispatch=action == "timing"))
        case _:
            sys.exit("usage: controller.py check|setup|{timing,replay} {pilot,peers}|replay setup")
