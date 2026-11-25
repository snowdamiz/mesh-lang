"""Seal and replay the claim-lane trial through the existing Linode controller."""

import json
from pathlib import Path
import statistics
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
PREVIOUS = ROOT / "tools/durable-jobs-bench/results/2026-11-24-dense-individual-hot-pool"
sys.path.insert(0, str(PREVIOUS / "logger-off-retry"))
import retry

base, dense, run, require = retry.base, retry.dense, retry.run, retry.require
ID = "2026-11-25-dense-claim-lane-1e70f18d"
STAGE = dense.BASE / "dense-claim-lane-1e70f18d"
ARTIFACT = STAGE / "artifacts/jobs"
OUT = HERE / "evidence/native"
QUEUE = base.matrix.QUEUE
SWITCH = "MESH_JOBS_CLAIM_LANE_LAUNCH"
OLD_STAGES, OLD_SETUP_RAW, ORIGINAL_MESH = base.stages, base.setup_raw, base.mesh
INSTALL = {"native-run", "adapter.py", "identity.py"}
ORIGINAL_SUMMARY = base.summarize
FLOOR = 0.05


def payload_files():
    files = {name: (HERE / name).read_bytes() for name in sorted(INSTALL | {"claim-lane.patch"})}
    files["proof-toolchain.sha256"] = retry.toolchain_pin_bytes()
    for path in sorted((HERE / "overlay").rglob("*")):
        if path.is_file():
            files[str(path.relative_to(HERE))] = path.read_bytes()
    return files


def pins():
    return {name: run.digest(HERE / name) for name in sorted(INSTALL)}


def packet(action):
    path = HERE / f"{action}.json"
    doc = run.read_json(path)
    expected = ID + ("-setup" if action == "setup" else "")
    require(doc["id"] == expected and doc["source_sha"] == base.REV
            and doc["build_free"] is (action == "timing"), "wrong controller request")
    for name, digest in doc["local_sha256"].items():
        require(run.digest(ROOT / name) == digest, f"sealed input changed: {name}")
    if action == "setup":
        require(dense.digest(retry.payload()) == doc["payload_sha256"], "payload differs")
    return path, doc


def plan():
    doc = run.read_json(HERE / "plan.json")
    first, second = doc["blocks"]
    labels = {*dense.CONTROLS, "hot", *base.PEERS}
    require(first["group"] == 1 and second["group"] == 2
            and set(first["order"]) == labels and len(first["order"]) == len(labels)
            and second["order"] == first["order"][::-1], "pilots must reverse every lane")
    require(doc["input_sha256"] == dense.INPUT
            and doc["gate"] == dict(minimum_faster_control_elapsed_fraction=FLOOR, minimum_groups=2,
                                   total_groups=2, require_earlier_last_effect=True), "gate changed")
    for block in (first, second):
        require(block["profile"] == "noop" and block["phases"] == ["warmup", "measured"], "wrong workload")
    return [first, second]


def stages(config, label, remote):
    files, inventory = OLD_STAGES(config, label if label in base.PEERS else "hot", remote)
    if label in base.PEERS:
        return files, inventory
    engine = inventory["engines"]["mesh"]
    engine["argv"].insert(2, f"{SWITCH}={int(label == 'hot')}")
    engine["verified_files"].update({
        str(STAGE / path): dense.digest(data)
        for path, data in payload_files().items()
        if path.startswith("overlay/replay/") and path.endswith(".py")
    })
    run.check_inventory(inventory)
    return files, inventory


def verify_held(get, elf):
    outcomes = {}
    for mode in ("off", "on", "lost-ack", "stop"):
        result = json.loads(get(f"semantic-root/held/{mode}/result.json"))
        require(result["executable_sha256"] == elf and result["mode"] == mode
                and result["shutdown"]["state"] == "stopped" and result["shutdown"]["drained"]
                and result["terminal"]["idle"] and result["grant"]["idle"]
                and result["during"]["first"]["state"] == "completed"
                and result["blocked"]["finished"] == 0
                and result["blocked"]["pending_claims"] > 0, f"held receipt proof failed: {mode}")
        overlap = mode in ("on", "lost-ack")
        require(bool(result["during"]["peer"]["attempts"]) == overlap, f"wrong live entry: {mode}")
        if overlap:
            require(len(result["overlap"]["undrained_jobs"]) == 2 and result["overlap"]["finished"] == 0,
                    "blocked slot went uncharged")
        outcomes[mode] = result
    return outcomes


def binding():
    source = retry.OLD_BIND()
    raw = OUT / "setup/raw.tar.gz"
    _, doc = packet("setup")
    with tarfile.open(raw) as archive:
        get = dense.archive_files(archive)
        require(get("SETUP-PASS") == b"" and get("native.exit").strip() == b"0", "native setup failed")
        installed = {name: pin for pin, name in (line.split() for line in get("installed.sha256").decode().splitlines())}
        require(installed == {name: dense.digest(data) for name, data in payload_files().items()},
                "native build inputs differ")
        manifest = get("jobs.json")
        require(get("mesh.json") == manifest, "mismatched staged runner manifest")
        built_runtime = get("runtime.sha256").split()[0].decode()
        originals = retry.old_manifests()
        for name in ("jobs", "witness"):
            reference = originals[name]
            actual = json.loads(get(f"{name}.json"))
            require(actual["runtime_sha256"] == built_runtime != reference["runtime_sha256"]
                    and actual["source_sha256"] == doc["payload_sha256"]
                    and all(actual[key] == value for key, value in reference.items()
                            if key not in {"runtime_sha256", "source_sha256", "files"}), f"build identity drift: {name}")
            files = {**reference["files"], "main.mpl": retry.FIXTURE if name == "jobs" else retry.witness_main_digest()}
            require({key: value for key, value in actual["files"].items() if key != "mesh"}
                    == {key: value for key, value in files.items() if key != "mesh"}, f"handler or producer drift: {name}")
            require(b"ELF64" in get(f"{name}-elf.txt") and b"Advanced Micro Devices X86-64" in get(f"{name}-elf.txt"),
                    "unexpected ELF")
        cases = retry.proof_controller()["CASES"]
        require(all(f"PASS {name}\n".encode() in get("semantic.log") for name in cases), "semantic check missing")
        verify_held(get, json.loads(manifest)["files"]["mesh"])
        require(all(f"PASS {mode}\n".encode() in get("controls.log") for mode in ("pause", "scale"))
                and all(f"PASS {mode}\n".encode() in get("grant-edges.log")
                        for mode in ("cancelled", "expired", "malformed")), "grant control proof missing")
    return dict(manifest=json.loads(manifest), manifest_sha256=dense.digest(manifest),
                proof_raw_sha256=source["proof_raw_sha256"])


def measured_mesh(get, log, phase, _hot, product, executable):
    retry.verify_dense_logs(get, log)
    return ORIGINAL_MESH(get, log, phase, True, product, executable)


def setup_raw(product):
    qualification = OLD_SETUP_RAW(product)
    diagnostics = {}
    with tarfile.open(OUT / "setup/raw.tar.gz") as archive:
        get = dense.archive_files(archive)
        for setting in ("0", "1"):
            ledger = json.loads(get(f"{setting}/ledger.json"))
            jobs_by_pid = {}
            for job in ledger["jobs"]:
                jobs_by_pid.setdefault(job["worker_pid"], {})[job["id"]] = job
            process_ids = [entry["pid"] for entry in json.loads(get(f"{setting}/process-identity.json"))[1:]]
            samples = []
            for i, pid in enumerate(process_ids):
                doc = json.loads(get(f"{setting}/grant-ages-{i}.json"))
                jobs = jobs_by_pid[pid]
                require(doc["dropped"] == 0 and len(doc["samples"]) == len(jobs)
                        and {row[0] for row in doc["samples"]} == set(jobs), "diagnostic job coverage differs")
                for job_id, confirmed, dispatch, scheduler in doc["samples"]:
                    require(confirmed > 0 and dispatch >= 0 and scheduler >= 0
                            and jobs[job_id]["claimed_us"] <= confirmed <= jobs[job_id]["entered_us"],
                            "invalid diagnostic age")
                    stamp_to_receipt = confirmed - jobs[job_id]["claimed_us"]
                    samples.append((stamp_to_receipt, dispatch, scheduler))
            require(len(samples) == dense.JOBS, "incomplete diagnostic population")
            diagnostics[setting] = {
                "jobs": len(samples),
                **{
                    name: {"median": statistics.median(row[index] for row in samples),
                           "max": max(row[index] for row in samples)}
                    for index, name in enumerate(("claim_stamp_to_client_receipt_us", "confirmed_to_spawn_ns",
                                                   "spawn_to_entry_ns"))
                },
            }
    return dict(qualification, diagnostic_ages=diagnostics)


def summarize(*args):
    result = ORIGINAL_SUMMARY(*args)
    if result["missing"]:
        decision = "incomplete"
    elif result["advances"]:
        decision = "advance"
    else:
        decision = "closed"
    return dict(result, pilot_decision=decision)


def configure():
    retry.HERE, retry.OUT, retry.REMOTE, retry.ID = HERE, OUT, OUT_REMOTE, ID
    retry.payload_files, retry.packet, retry.relink_binding = payload_files, packet, binding
    base.ID, base.STAGE, base.ARTIFACT, base.OUT = ID, STAGE, ARTIFACT, OUT
    base.request, base.bind_proof, base.setup_raw, base.adapter_files = packet, binding, setup_raw, pins
    base.stages, base.plan, base.mesh = stages, plan, measured_mesh
    base.summarize = summarize
    dense.PLAN = HERE / "plan.json"


OUT_REMOTE = dense.BASE / "runs" / STAGE.name


def check():
    blocks = plan()
    setup_doc, timing_doc = (packet(action)[1] for action in ("setup", "timing"))
    require(timing_doc["schedule"] == blocks and timing_doc["setup_payload_sha256"] == setup_doc["payload_sha256"],
            "schedule or stage binding differs")
    require(run.digest(HERE / "native-stage.tar") == setup_doc["payload_sha256"],
            "retained staged bytes differ")
    require(run.digest(HERE / "overlay/fixture/main.mpl") == retry.FIXTURE,
            "canonical runner changed")
    for filename in ("setup.sh", "native-run"):
        subprocess.run(["bash", "-n", HERE / filename], check=True)
    cfg = base.config({"manifest": {"files": {"mesh": "0" * 64}}, "manifest_sha256": "1" * 64})
    remote = run.REMOTE / "dense-claim-lane-offline"
    for label in base.PEERS:
        require(stages(cfg, label, remote) == OLD_STAGES(cfg, label, remote), f"peer inventory changed: {label}")
    files, candidate = stages(cfg, "hot", remote)
    candidate["engines"]["mesh"]["argv"][2] = f"{SWITCH}=0"
    for label in dense.CONTROLS:
        require((files, candidate) == stages(cfg, label, remote), f"{label} differs beyond switch")
    print("PASS canonical source, exact peer inventories, reversed pilots and sole same-ELF switch")


def seal():
    reference = run.read_json(PREVIOUS / "logger-off-retry/setup.json")
    local = dict(reference["local_sha256"])
    inputs = ["campaign.py", "setup.sh", "native-run", "plan.json", "claim-lane.patch", *sorted(INSTALL)]
    inputs += [name for name in payload_files() if name.startswith("overlay/")]
    local.update({str((HERE / name).relative_to(ROOT)): run.digest(HERE / name) for name in inputs})
    seal = dense.digest(retry.payload())
    full = str(HERE / "campaign.py")
    common = dict(source_sha=base.REV, after_existing_bookings=True, state="ready",
                  controller="native-controller-complete-post-wal-queue", cpus=[0, 1], ready=True,
                  fixture_sha256=retry.FIXTURE, input_sha256=dense.INPUT,
                  operator_id="dense-claim-lane-implementation", local_sha256=local,
                  check=f"python3 -B {full} check")
    setup = dict(common, id=f"{ID}-setup", kind="setup", build_free=False, requested_window_minutes=25,
                 payload_sha256=seal, command=f"python3 -B {full} setup",
                 setup_command=f"python3 -B {full} setup",
                 evidence_destination=str(OUT / "setup"),
                 scope="Isolated released source build, canonical relink, held-ack semantics and untimed grant-age diagnostics")
    timing = dict(common, id=ID, kind="timing", build_free=True, pilot_window_minutes=12, pilot_groups=[1, 2],
                  setup_payload_sha256=seal, setup_dependency=setup["id"],
                  command=f"python3 -B {full} timing 1 2", timing=f"python3 -B {full} timing 1 2",
                  replay=f"python3 -B {full} replay", schedule=plan(),
                  evidence_destination=str(OUT),
                  scope="Two reversed matched original-producer DENSE pilots. Close if either group misses the gate.")
    for action, doc in (("setup", setup), ("timing", timing)):
        run.write_json(HERE / f"{action}.json", doc)
    (HERE / "native-stage.tar").write_bytes(retry.payload())


if __name__ == "__main__":
    configure()
    action = sys.argv[1]
    if action == "seal":
        seal()
    elif action == "check":
        check()
    elif action == "setup":
        retry.setup()
    else:
        require((action == "timing" and sys.argv[2:] == ["1", "2"]) or action == "replay",
                "only run the sealed pilot")
        sys.exit(base.main())
