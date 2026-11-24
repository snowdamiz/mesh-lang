"""Bind and replay the current public ELF. Only the assigned native controller dispatches."""
import argparse
from copy import deepcopy
from datetime import datetime, timezone
import fcntl
import gzip
from io import BytesIO
import json
from pathlib import Path
import re
import runpy
import subprocess
import sys
import tarfile
import tempfile

HERE = Path(__file__).resolve().parent
RESULTS = HERE.parent
sys.path.insert(0, str(RESULTS / "2026-11-24-dense-product-d2-transfer"))
import transfer as dense
from interleave import HOST_IDLE

from score import intervals, lifecycle, mesh

run, matrix, require = dense.run, dense.matrix, dense.require
ID = HERE.name
SOURCE = RESULTS / "2026-11-24-qualified-hot-pool"
PROOF = SOURCE / "evidence/jobs"
REV = "1e70f18d624a841740d3bbb3d0fd09b269257f68"
PROOF_SHA = "78468180f0c0be9b2b17533b54dc8cbd69e3982d84b239457b15e5beb018b530"
PROOF_STAGE = dense.BASE / "source-trusted-1e70f18d-jobs-78468180f0c0"
ARTIFACT = PROOF_STAGE / "artifacts/jobs"
STAGE = dense.BASE / "dense-individual-hot-pool-1e70f18d"
REMOTE_SETUP = dense.BASE / "runs" / STAGE.name
OUT = HERE / "evidence"
INSTALL_FILES = ("native-run", "adapter.py", "identity.py")
PEERS = dense.PEERS
MESH = (*dense.CONTROLS, "hot")
METRICS = ("submit_us", "drain_us", "middle_effect_us", "middle_settlement_us")
PLAN = HERE / "plan.json"
TEMPLATE = RESULTS / "2026-11-24-bulk-d2-short-peers/short.json"
HARNESS_ARCHIVE = RESULTS / "2026-11-23-set-based-enqueue/native/bulk-enqueue-ce5e551d079e.tar.gz"
base_stages, base_score = matrix.stages, matrix.score_arm


def adapter_files():
    return {name: run.digest(HERE / name) for name in INSTALL_FILES}


def request(action):
    path = HERE / f"{action}.json"
    packet = run.read_json(path)
    expected = f"{ID}-setup" if action == "setup" else ID
    require(packet["id"] == expected and packet["build_free"] is True and packet["source_sha"] == REV,
            "unexpected controller request")
    for name, pin in packet["local_sha256"].items():
        require(run.digest(dense.ROOT / name) == pin, f"sealed input differs: {name}")
    return path, packet


def plan():
    data = run.read_json(PLAN)
    require(data["modes"] == dict.fromkeys(dense.CONTROLS, False) | {"hot": True}, "unexpected checkout switches")
    return dense.schedule(data)


def proof_inputs():
    return run.read_json(SOURCE / "jobs.json")


def bind_proof():
    packet = proof_inputs()
    require(packet["source_sha"] == REV and packet["archive_sha256"] == PROOF_SHA, "wrong Jobs proof request")
    grant = run.read_json(PROOF / "grant.json")
    kind, label, start, end = grant["event"]
    require(kind == "SETUP" and label == packet["id"] and grant["seal"] == run.digest(SOURCE / "jobs.json")
            and datetime.fromisoformat(start) < datetime.fromisoformat(end), "raw Jobs proof lacks its sealed SETUP")
    raw = PROOF / "raw.tar.gz"
    sys.path.insert(0, str(SOURCE))
    try:
        proof = runpy.run_path(str(SOURCE / "controller.py"))
        proof["check"](packet)
        verified = proof["verify"](packet, raw)
    finally:
        sys.path.pop(0)
    raw_hash = run.digest(raw)
    require(run.read_json(PROOF / "verified.json") == dict(verified, source=REV, raw_sha256=raw_hash),
            "Jobs proof receipt differs from archived replay")
    with tarfile.open(raw) as archive:
        get = dense.archive_files(archive)
        manifest = get("jobs.json")
        elf_header = get("jobs-elf.txt")
        require(b"ELF64" in elf_header and b"Advanced Micro Devices X86-64" in elf_header,
                "Jobs proof did not record an x86-64 ELF")
        for case in ("short-set-grant", "short-hot-grant"):
            receipt = json.loads(get(f"semantic-root/semantic/{case}/result.json"))
            final = receipt["final"]
            all_xids = [receipt["producer_xid"], next(iter(receipt["held"].values()))["xid"]]
            for job in final.values():
                require(job["state"] == "completed" and job["attempt"] == 1 and job["token"] is None
                        and len(job["attempts"]) == 1, f"{case}: unexpected public terminal receipt")
                all_xids.extend([*job["attempts"], job["effect"], job["xid"]])
            require(len(final) == 2 and len({int(x) % (2**32) for x in all_xids}) == 8,
                    f"{case}: enqueue, grant, application and terminal transactions overlapped")
    parsed = json.loads(manifest)
    require(parsed["compiler_sha256"] == dense.COMPILER_SHA and parsed["files"]["mesh"] == verified["elf"],
            "wrong exact Jobs proof ELF or compiler")
    return dict(manifest=parsed, manifest_sha256=dense.digest(manifest), proof_raw_sha256=raw_hash,
                proof_receipt_sha256=run.digest(PROOF / "verified.json"), proof=verified)


def frozen_pins():
    with tarfile.open(SOURCE / "source.tar.gz") as archive:
        get = dense.archive_files(archive)
        files = ("source/scripts/fixtures/jobs-role-grants.sql", "overlay/replay/backend.py",
                 "overlay/frozen-harness.sha256")
        pins = {str(PROOF_STAGE / name): dense.digest(get(name)) for name in files}
        pins.update({str(dense.FROZEN / name): seal for seal, name in
                     (line.split() for line in get("overlay/frozen-harness.sha256").decode().splitlines())})
    return pins


def config(binding):
    cfg = dense.configurations({})
    cfg["description"] = "Current public individual-terminal dense checkout transfer on CPUs 0/1"
    cfg["product"] = binding
    return cfg


def stages(cfg, label, remote):
    if label in PEERS:
        files, inventory = base_stages(cfg, label, remote)
        engine = cfg["lanes"][label]["engine"]
        inventory["engines"][engine]["verified_files"][dense.POSTGRES] = dense.PG_SHA
        run.check_inventory(inventory)
        return files, inventory
    binding = cfg["product"]
    manifest = binding["manifest"]
    spec = deepcopy(run.read_json(TEMPLATE)["lanes"]["bulk"]["inventory"]["engines"]["mesh"])
    spec.pop("run_as", None)
    verified = {path: pin for path, pin in spec["verified_files"].items() if path.startswith(str(dense.FROZEN) + "/")}
    verified.update(frozen_pins())
    verified.update({str(ARTIFACT / name): seal for name, seal in manifest["files"].items()})
    verified.update({str(STAGE / name): pin for name, pin in adapter_files().items()})
    verified.update({str(ARTIFACT / "mesh.json"): binding["manifest_sha256"],
                     dense.POSTGRES: dense.PG_SHA, str(remote / "worker.py"): run.digest(dense.SUITE / "worker.py")})
    spec.update(argv=["/usr/bin/env", f"HOT_CHECKOUT={int(label == 'hot')}",
                      f"PRODUCT_ELF_SHA256={manifest['files']['mesh']}",
                      f"PRODUCT_MANIFEST_SHA256={binding['manifest_sha256']}",
                      "/bin/bash", str(STAGE / "native-run"), "mesh"],
                artifact_sha256=binding["manifest_sha256"], version=REV, verified_files=verified)
    inventory = dict(cfg["inventory"], engines={**cfg["inventory"]["engines"], "mesh": spec})
    run.check_inventory(inventory)
    return [dense.SUITE / "worker.py"], inventory


def grant(path):
    entries = re.findall(r"^(SETUP|TIMING|RELEASED|HOLD)\s+(\S+)\s+(\S+)(?:[ \t]+(\S+))?",
                        (matrix.QUEUE / "STATUS.md").read_text(), re.MULTILINE)
    mode, label, start, end = entries[-1]
    packet = run.read_json(path)
    expected = "SETUP" if path.name == "setup.json" else "TIMING"
    require((mode, label) == (expected, packet["id"]) and end
            and datetime.fromisoformat(start) <= datetime.now(timezone.utc) < datetime.fromisoformat(end)
            and path.read_bytes() == (matrix.QUEUE / f"{label}.json").read_bytes(), "active sealed assignment required")
    return dict(mode=mode, id=label, start=start, end=end, request_sha256=run.digest(path))


def setup_raw(binding):
    folder = OUT / "setup"
    raw = folder / "raw.tar.gz"
    receipt = run.read_json(folder / "grant.json")
    path, packet = request("setup")
    require(receipt["mode"] == "SETUP" and receipt["id"] == packet["id"]
            and receipt["request_sha256"] == run.digest(path)
            and datetime.fromisoformat(receipt["start"]) < datetime.fromisoformat(receipt["end"]),
            "adapter setup grant does not match the sealed request")
    require(run.read_json(folder / "status.json")["exit_code"] == 0, "adapter setup did not exit successfully")
    with tarfile.open(raw) as archive:
        get = dense.archive_files(archive)
        require(get("SETUP-PASS") == b"" and dense.digest(get("mesh.json")) == binding["manifest_sha256"],
                "setup did not run from the exact proof manifest")
        require({name: pin for pin, name in
                 (line.split() for line in get("adapter.sha256").decode().splitlines())} == adapter_files(),
                "installed adapter bytes differ")
        arms = {}
        for arm in ("0", "1"):
            arms[arm] = mesh(lambda name: json.loads(get(f"{arm}/{name}.json")),
                             lambda name: get(f"{arm}/{name}").decode(), "diagnostic", arm == "1",
                             binding, ARTIFACT / "mesh")
    return dict(arms=arms, raw_sha256=run.digest(raw), jobs_proof=binding, files=adapter_files())


def setup(path, binding):
    cfg = config(binding)
    with (matrix.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        receipt = grant(path)
        remaining = (datetime.fromisoformat(receipt["end"]) - datetime.now(timezone.utc)).total_seconds() - 60
        require(remaining >= 300, "insufficient SETUP time")
        folder = OUT / "setup"
        folder.mkdir(parents=True, exist_ok=False)
        run.write_json(folder / "grant.json", receipt)
        stdin = BytesIO()
        with tarfile.open(fileobj=stdin, mode="w") as archive:
            for name in INSTALL_FILES:
                content = (HERE / name).read_bytes()
                member = tarfile.TarInfo(name)
                member.size = len(content)
                member.mode = 0o644
                archive.addfile(member, BytesIO(content))
        pins = [value for name, pin in adapter_files().items() for value in (pin, name)]
        frozen = frozen_pins()
        proof_pins = [frozen[str(PROOF_STAGE / name)] for name in ("overlay/replay/backend.py",
                      "source/scripts/fixtures/jobs-role-grants.sql", "overlay/frozen-harness.sha256")]
        code = None
        try:
            with (folder / "setup.log").open("xb") as log:
                code = run.ssh(["bash", "-c", (HERE / "setup.sh").read_text(), "--", *cfg["host"],
                                binding["manifest"]["files"]["mesh"], binding["manifest_sha256"], *proof_pins, *pins],
                               input=stdin.getvalue(), stdout=log, stderr=log, timeout=min(remaining, 600)).returncode
        finally:
            run.write_json(folder / "status.json", {"exit_code": code})
            with (folder / "raw.tar.gz").open("xb") as raw, (folder / "export.log").open("xb") as log:
                exported = run.ssh(["bash", "-c", HOST_IDLE + '\nexec tar -czf - -C "$1" .',
                                    "--", REMOTE_SETUP], stdout=raw, stderr=log, timeout=55)
        require(code == 0 and exported.returncode == 0, "SETUP failed, retain raw evidence and reconcile fences")
        run.write_json(folder / "verified.json", setup_raw(binding))


def score_arm(folder, phase, label, cfg, remote):
    _, inventory = stages(cfg, label, remote)
    engine = cfg["lanes"][label]["engine"]
    path = folder / "raw.tar.gz"
    with tarfile.open(path) as raw:
        def get(name):
            return run.file_json(raw, f"adapter/{name}.json")

        ledger = get("ledger")
        if label in PEERS:
            score = base_score(folder, engine, 2, phase, inventory, cfg)
            return dict(score, **intervals(ledger), terminal_xids=lifecycle(ledger, individual=False),
                        application_xids=2 * dense.JOBS, last_effect_us=max(j["effect_committed_us"] for j in ledger["jobs"]))

        invocation = run.file_json(raw, "invocation.json")
        request = run.read_json(folder / "request.json")
        require(all(invocation[k] == request[k] == value for k, value in
                    dict(profile="noop", phase=phase, engine=engine, attempt=request["attempt"]).items())
                and run.file_json(raw, "status.json")["exit_code"] == 0, "wrong or failed invocation")
        host = run.file_json(raw, "host.json")
        require([host["boot_id"], host["kernel"], host["pg_mount"]["maj:min"]] == cfg["host"]
                and host["affinity"] == dense.CPUS and len(host["available_cpus"]) == 4
                and host["verified_files"] == inventory["engines"]["mesh"]["verified_files"], "host or pins differ")
        score = mesh(get, lambda name: dense.text(raw, f"adapter/{name}"), phase, label == "hot",
                     cfg["product"], ARTIFACT / "mesh")
        with raw.extractfile("host-samples.jsonl") as stream:
            score.update(run.sample_window(host, [json.loads(line) for line in stream], score))
    return dict(score, **dense.host_cpu(folder, score, cfg["clock_ticks"]), raw_sha256=run.digest(path))


def replay_block(path, block, cfg, remotes):
    require(run.read_json(path / "order.json") == block, "rotation order changed")
    return {f"{phase}-{label}": score_arm(folder, phase, label, cfg, run.REMOTE / remotes[label])
            for phase, label, _, folder in matrix.paths(path, cfg, block)}


def summarize(directory, cfg, blocks, pins):
    result = dense.summarize(directory, cfg, blocks, pins)
    measured = [{label: block["scores"][f"measured-{label}"] for label in cfg["lanes"]}
                for block in result["blocks"].values()]
    result["elapsed_us"] = {}
    result["paired_hot_minus_us"] = {}
    if measured:
        for label in cfg["lanes"]:
            result["elapsed_us"][label] = {
                key: matrix.distribution([row[label][key] for row in measured]) for key in METRICS
            }
            if label == "hot":
                continue
            result["paired_hot_minus_us"][label] = {
                key: dict(matrix.distribution(deltas), hot_wins=sum(x < 0 for x in deltas))
                for key in METRICS
                for deltas in [[row["hot"][key] - row[label][key] for row in measured]]
            }
    return result


def check():
    blocks = plan()
    path, packet = request("timing")
    request("setup")
    require(packet["schedule"] == blocks, "request schedule differs")
    require(run.read_json(PLAN)["gate"] == run.read_json(dense.PLAN)["gate"],
            "advancement gate differs from the prior dense transfer")
    data = gzip.decompress((HERE / "batch.json.gz").read_bytes())
    require(dense.digest(dense.digest(data).encode()) == dense.INPUT, "portable dense input differs")
    for name in ("setup.sh", "native-run"):
        subprocess.run(["bash", "-n", HERE / name], check=True)
    # Exercise all frozen harness anchors on the exact archived sources. No process or DB workload.
    with tempfile.TemporaryDirectory(dir=HERE) as temporary:
        root = Path(temporary)
        for name, prefix in [(HARNESS_ARCHIVE, "ordinary-v2/"), (SOURCE / "source.tar.gz", "overlay/")]:
            with tarfile.open(name) as archive:
                for member in archive:
                    if member.isfile() and member.name.startswith(prefix) and member.name.endswith((".py", ".json")):
                        path = root / member.name
                        require(path.is_relative_to(root) and ".." not in path.parts, "invalid harness archive path")
                        path.parent.mkdir(parents=True, exist_ok=True)
                        path.write_bytes(archive.extractfile(member).read())
        subprocess.run(["env", f"PRODUCT_SOURCE={root / 'source'}", f"PRODUCT_FROZEN_HARNESS={root / 'ordinary-v2'}",
                        sys.executable, "-B", "-c", "import adapter; adapter.install()"], cwd=HERE, check=True)
    placeholder = {"manifest": {"files": {"mesh": "0" * 64}}, "manifest_sha256": "1" * 64}
    cfg = config(placeholder)
    remote = run.REMOTE / "dense-offline"
    peers = dense.configurations({})
    for label in PEERS:
        # The prior dense pack and this one must yield the same peer inventory and staged paths.
        require(stages(cfg, label, remote) == dense.stages(peers, label, remote), f"peer inventory drift: {label}")
    off = stages(cfg, "checked-a", remote)
    require(off == stages(cfg, "checked-b", remote), "checked controls differ")
    off[1]["engines"]["mesh"]["argv"][1] = "HOT_CHECKOUT=1"
    require(off == stages(cfg, "hot", remote), "hot changes more than the checkout option")
    print("Six balanced orders, three equal peer inventories, one mesh invocation delta, canonical workload and exact adapter anchors verified.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "bind", "setup", "timing", "replay"))
    parser.add_argument("groups", type=int, nargs="*", choices=range(1, 7))
    args = parser.parse_args()
    if args.action == "check":
        check()
        return 0
    setup_path, _ = request("setup")
    timing_path, packet = request("timing")
    binding = bind_proof()
    if args.action == "bind":
        print(json.dumps(binding, indent=2, sort_keys=True))
        return 0
    if args.action == "setup":
        setup(setup_path, binding)
        return 0
    qualification = setup_raw(binding)
    require(run.read_json(OUT / "setup/verified.json") == qualification, "setup replay differs")
    release = OUT / "setup/controller-release.json"
    document = run.read_json(release)
    require(document["id"] == f"{ID}-setup" and document["exit_status"] == 0, "SETUP has no controller release")
    blocks = plan()
    require(packet["schedule"] == blocks, "timing schedule differs")
    cfg = config(binding)
    pins = dict(packet["local_sha256"], request=run.digest(timing_path), setup=qualification["raw_sha256"],
                setup_release=run.digest(release), proof=binding["proof_raw_sha256"])
    directory = OUT / "native"
    matrix.stages, matrix.replay_block, matrix.summarize = stages, replay_block, summarize
    matrix.ERRORS += (AssertionError,)
    if args.action == "timing":
        require(args.groups in ([1, 2], [3, 4, 5, 6]), "use pilot 1 2 or continuation 3 4 5 6")
        if args.groups[0] == 3:
            require(not {1, 2} & set(summarize(directory, cfg, blocks, pins)["missing"]), "pilot must reconcile")
        matrix.grant = lambda _: grant(timing_path)
        matrix.SERIES[ID] = ID
        dispatch = run.dispatch_arm

        def checked_dispatch(path, remote, block, phase, engine):
            dispatch(path, remote, block, phase, engine)
            folder = path / f"{phase}-{engine}"
            require(run.read_json(folder / "dispatch.json").get("exit_code") == 0, "native arm failed")
            score_arm(folder, phase, path.name.removeprefix(phase + "-"), cfg, remote)

        run.dispatch_arm = checked_dispatch
        matrix.dispatch(directory, cfg, ID, blocks, pins, args.groups)
    result = summarize(directory, cfg, blocks, pins)
    print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))
    pending = args.groups if args.action == "timing" else range(1, 7)
    return bool(set(pending) & set(result["missing"]))


if __name__ == "__main__":
    sys.exit(main())
