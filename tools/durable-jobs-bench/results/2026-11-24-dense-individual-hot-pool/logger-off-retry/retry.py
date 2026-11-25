"""Qualify a logger-off relink once, then reuse the frozen dense timing controller."""
from collections import Counter
from datetime import datetime, timezone
import fcntl
from io import BytesIO
import json
from pathlib import Path
import re
import runpy
import subprocess
import sys
import tarfile

from logger import verify_dense_logs, verify_runner_log

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import controller as base

dense, run, require = base.dense, base.run, base.require
ID = "2026-11-24-dense-individual-logger-off-retry"
STAGE = dense.BASE / "dense-individual-logger-off-1e70f18d"
ARTIFACT = STAGE / "artifacts/jobs"
REMOTE = dense.BASE / "runs" / STAGE.name
OUT = HERE / "evidence"
OLD_FAILURE = HERE.parent / "evidence/setup/raw.tar.gz"
SOURCE = base.SOURCE / "source.tar.gz"
PROOF_RAW = base.PROOF / "raw.tar.gz"
EXACT_RUNTIME = "c2e0ae4a4b61b2acdc88fc8bc65d0ab0f1c4e607e41daf4bfa3b288d15a7bcc2"
FIXTURE = "97b8e75f8568191ac2b3a3160ce9c40d0eebc822ba2ec4661259510734816661"
EVENT_OPTION = br',\"event_log\":true'
OLD_BIND, BASE_MESH = base.bind_proof, base.mesh
ADAPTER_FILES = {
    "native-run": HERE / "native-run",
    "adapter.py": HERE.parent / "adapter.py",
    "identity.py": HERE.parent / "identity.py",
}


def adapter_pins():
    return {name: run.digest(path) for name, path in ADAPTER_FILES.items()}


def packet(action):
    path = HERE / f"{action}.json"
    document = run.read_json(path)
    identifier = f"{ID}-setup" if action == "setup" else ID
    require(document["id"] == identifier and document["build_free"] is (action == "timing")
            and document["source_sha"] == base.REV and document["runtime_sha256"] == EXACT_RUNTIME
            and document["fixture_sha256"] == FIXTURE, "wrong logger-off request")
    for name, pin in document["local_sha256"].items():
        require(run.digest(dense.ROOT / name) == pin, f"sealed input differs: {name}")
    return path, document


def toolchain_pin_bytes():
    with tarfile.open(PROOF_RAW) as archive:
        content = dense.archive_files(archive)("toolchain.txt")
    pins = [line for line in content.splitlines() if re.fullmatch(rb"[0-9a-f]{64}  /[^ ]+", line)]
    require(len(pins) == 7, "incomplete original toolchain pins")
    return b"\n".join(pins) + b"\n"


def payload_files():
    return {name: path.read_bytes() for name, path in ADAPTER_FILES.items()} | {
        "main.mpl": (HERE / "main.mpl").read_bytes(),
        "proof-toolchain.sha256": toolchain_pin_bytes(),
    }


def payload():
    files = payload_files()
    files["installed.sha256"] = "".join(f"{dense.digest(data)}  {name}\n" for name, data in files.items()).encode()
    buffer = BytesIO()
    with tarfile.open(fileobj=buffer, mode="w") as archive:
        for name, data in files.items():
            member = tarfile.TarInfo(name)
            member.size, member.mode = len(data), 0o644
            archive.addfile(member, BytesIO(data))
    return buffer.getvalue()


def proof_controller():
    sys.path.insert(0, str(base.SOURCE))
    try:
        return runpy.run_path(str(base.SOURCE / "controller.py"))
    finally:
        sys.path.pop(0)


def old_manifests():
    with tarfile.open(PROOF_RAW) as archive:
        get = dense.archive_files(archive)
        return {name: json.loads(get(f"{name}.json")) for name in ("jobs", "witness")}


def verify_relink_manifest(old, new, main_sha, package_sha):
    require(new["source_sha256"] == package_sha and new["runtime_sha256"] == EXACT_RUNTIME
            and {k: v for k, v in new.items() if k not in {"files", "source_sha256"}} ==
                {k: v for k, v in old.items() if k not in {"files", "source_sha256"}},
            "the relink changed the exact runtime, compiler or toolchain")
    changed = {name for name in old["files"].keys() | new["files"].keys()
               if old["files"].get(name) != new["files"].get(name)}
    require(changed == {"main.mpl", "mesh"} and new["files"]["main.mpl"] == main_sha,
            "the relink changed files beyond the logger-off main and executable")


def witness_main_digest():
    # Same anchor substitutions as the sealed witness builder, without linking anything.
    main = (HERE / "main.mpl").read_bytes()
    for old, new in (
        (b"import realtime_us, worker_pid", b"import realtime_us, worker_pid, arm_teardown"),
        (b"fn handle(context :: String) -> String do\n",
         b'fn handle(context :: String) -> String do\n  if arm_teardown() != 0 do panic("cannot register teardown witness") end\n'),
    ):
        require(main.count(old) == 1, "witness builder anchor differs")
        main = main.replace(old, new)
    return dense.digest(main)


def verify_semantics(raw, package_sha, witness_sha):
    proof = proof_controller()
    observed = proof["verify"](dict(base.proof_inputs(), archive_sha256=package_sha), raw)
    with tarfile.open(raw) as archive:
        get = dense.archive_files(archive)
        for name in proof["CASES"]:
            require(f"PASS {name}\n".encode() in get("semantic.log"), f"semantic case {name} did not finish")
        def case(name):
            return json.loads(get(f"semantic-root/semantic/{name}/result.json"))

        for key in ("short-set-grant", "short-hot-grant", "mixed-checked-cursor"):
            receipt = case(key)
            xids = [receipt["producer_xid"], next(iter(receipt["held"].values()))["xid"]]
            for job in receipt["final"].values():
                require(job["state"] == "completed" and job["attempt"] == 1 and job["token"] is None
                        and len(job["attempts"]) == 1, f"{key}: wrong terminal or app receipt")
                xids.extend([*job["attempts"], job["effect"], job["xid"]])
            require(len(receipt["final"]) == 2 and len({int(x) % 2**32 for x in xids}) == 8,
                    f"{key}: overlapping enqueue, grant, application or terminal XIDs")
        claim = case("ambiguous-claim-commit")
        require(claim["wire"]["receipt_dropped"] and claim["wire"]["idle"]
                and claim["held"]["effect"] is None and not claim["held"]["attempts"]
                and claim["done"]["attempt"] == 2 and len(claim["done"]["attempts"]) == 1
                and claim["first_app_commit_ms"] >= claim["held"]["expiry"], "claim receipt loss differs")
        for name, forced in (("teardown-release", False), ("teardown-kill", True)):
            row = case(name)
            held, done = row["held"], row["done"]
            require(row["forced"] is forced and row["runner_sha256"] == witness_sha
                    and held["state"] == "executing" and held["effect"] and len(held["attempts"]) == 1
                    and (done == held if forced else done["state"] == "completed"
                         and done["effect"] == held["effect"] and done["attempts"] == held["attempts"]
                         and done["xid"] != held["xid"]), f"{name}: teardown receipt differs")
    return observed


def relink_binding():
    old = OLD_BIND()
    _, document = packet("setup")
    raw = OUT / "setup/raw.tar.gz"
    require(run.read_json(OUT / "setup/status.json") == {"exit_code": 0}, "logger-off SETUP failed")
    with tarfile.open(raw) as archive:
        get = dense.archive_files(archive)
        require(get("native.exit").strip() == b"0" and get("SETUP-PASS") == b""
                and get("runtime.sha256").split()[0].decode() == EXACT_RUNTIME
                and get("runtime-origin").strip() in (b"retained", b"rebuilt"), "SETUP did not qualify")
        expected = {name: dense.digest(data) for name, data in payload_files().items()}
        actual = {name: pin for pin, name in (line.split() for line in get("installed.sha256").decode().splitlines())}
        require(actual == expected, "native input seal differs")
        originals = old_manifests()
        for name, main in (("jobs", FIXTURE), ("witness", witness_main_digest())):
            manifest = json.loads(get(f"{name}.json"))
            verify_relink_manifest(originals[name], manifest, main, document["payload_sha256"])
            require(b"ELF64" in get(f"{name}-elf.txt") and b"Advanced Micro Devices X86-64" in get(f"{name}-elf.txt"),
                    f"{name}: wrong native ELF architecture")
        manifest = get("mesh.json")
        require(manifest == get("jobs.json"), "dense launcher bound a different manifest")
        witness = json.loads(get("witness.json"))["files"]["mesh"]
        origin = get("runtime-origin").decode().strip()
    semantic = verify_semantics(raw, document["payload_sha256"], witness)
    require(json.loads(manifest)["files"]["mesh"] == semantic["elf"] and semantic["runtime"] == EXACT_RUNTIME,
            "relocated semantic proof used a different ELF or runtime")
    return dict(manifest=json.loads(manifest), manifest_sha256=dense.digest(manifest),
                proof_raw_sha256=old["proof_raw_sha256"], prior_elf=old["manifest"]["files"]["mesh"],
                relink_semantic=semantic, runtime_origin=origin)


def scored_mesh(get, log, phase, hot, binding, executable):
    verify_dense_logs(get, log)
    return BASE_MESH(get, log, phase, hot, binding, executable)


def configure():
    base.ID, base.STAGE, base.OUT, base.ARTIFACT = ID, STAGE, OUT, ARTIFACT
    base.request, base.adapter_files, base.bind_proof, base.mesh = packet, adapter_pins, relink_binding, scored_mesh


def setup():
    path, document = packet("setup")
    stage = payload()
    require(dense.digest(stage) == document["payload_sha256"], "setup payload differs from request")
    binding = OLD_BIND()
    release = run.read_json(base.PROOF / "controller-release.json")
    require(release["id"] == base.proof_inputs()["id"] and release["exit_status"] == 0,
            "original Jobs proof has no successful controller release")
    with (base.matrix.SUITE / ".timing.lock").open("rb") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        receipt = base.grant(path)
        remaining = (datetime.fromisoformat(receipt["end"]) - datetime.now(timezone.utc)).total_seconds() - 90
        require(remaining >= 600, "insufficient SETUP time")
        folder = OUT / "setup"
        folder.mkdir(parents=True, exist_ok=False)
        run.write_json(folder / "grant.json", receipt)
        code = None
        try:
            with (folder / "setup.log").open("xb") as log:
                code = run.ssh(["bash", "-c", (HERE / "setup.sh").read_text(), "--", *base.config(binding)["host"],
                                document["payload_sha256"], binding["manifest"]["files"]["mesh"],
                                binding["manifest_sha256"]], input=stage, stdout=log, stderr=log, timeout=remaining).returncode
        finally:
            run.write_json(folder / "status.json", {"exit_code": code})
            with (folder / "raw.tar.gz").open("xb") as raw, (folder / "export.log").open("xb") as log:
                exported = run.ssh(["bash", "-c", base.HOST_IDLE + '\nexec tar -czf - -C "$1" .',
                                    "--", REMOTE], stdout=raw, stderr=log, timeout=55)
        require(code == 0 and exported.returncode == 0, "preserve failed SETUP and reconcile the fences")
        run.write_json(folder / "verified.json", base.setup_raw(relink_binding()))


def failure_analysis():
    arms = []
    with tarfile.open(OLD_FAILURE) as archive:
        get = dense.archive_files(archive)
        failure = json.loads(get("0/failure.json"))
        require(failure["type"] == "FixtureError" and "invalid event buffer report" in failure["message"]
                and any(entry[2] == "mesh_event_buffer_report" for entry in failure["stage"]),
                "unexpected original failure")
        for i in range(2):
            text = get(f"0/runner-{i}.log").decode()
            lines = text.splitlines()
            report = json.loads(lines[-1])
            by_event = Counter(json.loads(line)["event"] for line in lines[1:-1])
            try:
                verify_runner_log(text)
            except ValueError as error:
                require("active event telemetry" in str(error), "original runner did not fail for live telemetry")
            else:
                raise ValueError("original logger-on arm passed the off check")
            arms.append(dict(runner=i, lines=len(lines), events=dict(by_event), claimed=report["claimed"],
                             event_buffer_bytes=report["event_buffer_bytes"]))
    return arms


def check():
    setup_path, stage = packet("setup")
    _, timing = packet("timing")
    require(dense.digest(payload()) == stage["payload_sha256"] and timing["schedule"] == base.plan(),
            "setup payload or measured schedule changed")
    with tarfile.open(SOURCE) as archive:
        original = dense.archive_files(archive)("overlay/fixture/main.mpl")
    recovered = (HERE / "main.mpl").read_bytes()
    require(original.count(EVENT_OPTION) == 1 and original.replace(EVENT_OPTION, b"") == recovered
            and dense.digest(recovered) == FIXTURE, "fixture delta exceeds disabling the per-job logger")
    binding = OLD_BIND()
    require(binding["manifest"]["runtime_sha256"] == EXACT_RUNTIME, "wrong original runtime")
    for name in ("native-run", "setup.sh"):
        subprocess.run(["bash", "-n", HERE / name], check=True)
    restored = (HERE.parent / "native-run").read_text().replace(
        "dense-individual-hot-pool-1e70f18d", STAGE.name).replace(
        "NATIVE_JOBS_ARTIFACTS=$proof/artifacts/jobs", "NATIVE_JOBS_ARTIFACTS=$here/artifacts/jobs")
    require(restored == (HERE / "native-run").read_text(), "launcher delta exceeds its stage and ELF path")
    print(json.dumps(dict(failed_raw_sha256=run.digest(OLD_FAILURE), failure=failure_analysis(),
                          fixture_sha256=FIXTURE, exact_runtime_sha256=EXACT_RUNTIME,
                          setup_request_sha256=run.digest(setup_path)), indent=2, sort_keys=True))


if __name__ == "__main__":
    action = sys.argv[1]
    if action == "check":
        check()
    elif action == "setup":
        configure()
        setup()
    elif action in ("timing", "replay"):
        configure()
        sys.exit(base.main())
    else:
        raise ValueError("expected check, setup, timing or replay")
