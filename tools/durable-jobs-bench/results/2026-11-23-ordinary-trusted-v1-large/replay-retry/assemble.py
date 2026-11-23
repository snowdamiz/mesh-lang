#!/usr/bin/env python3
"""Compose one large adapter from retained V1 and direct-producer sources."""

from hashlib import file_digest
import importlib.util
import json
from pathlib import Path
import shutil
import tarfile

LANE = Path(__file__).resolve().parents[1]
ROOT = LANE.parents[1]
BASE = "/var/lib/durable-jobs-bench/native"
V1 = ROOT / "target/ordinary-trusted-mesh-queue/ordinary-trusted-v1.tar.gz"
V1_SHA = "e742644d89b872a41b4d9d241091a2349491f3de6d393fe77c4cbdbc05f6a58c"
DIRECT_PREPARE_SHA = "ecd26276c21d38b5600ac5ddd8a00b2a4cf8f25aefa96ec5f818f3b493a948dc"
NATIVE = "tools/durable-jobs-bench/native"
TRIAL = "proof/jobs-parity/linux-regression"
POLICIES = ("external", "default")


def digest(path):
    with path.open("rb") as stream:
        return file_digest(stream, "sha256").hexdigest()


def read(name):
    return json.loads((LANE / "inputs" / name).read_text())


def choose(config, label):
    lane = config["lanes"][label]
    return lane["inventory"]["engines"][lane["engine"]]


def write(path, data):
    with path.open("x") as stream:
        json.dump(data, stream, sort_keys=True, indent=2)
        stream.write("\n")


def native_pins():
    v1_config, v1_build = read("v1-native.json"), read("v1-build.json")
    direct = choose(read("direct-file.json"), "mesh-direct-file")
    ordinary = choose(v1_config, "ordinary")
    external = read("symmetric-external.json")
    protected = choose(external, "mesh-direct-external")
    if v1_config["host"] != external["host"] or v1_config["cpus"] != external["cpus"] or external["cpus"] != [0, 1]:
        raise ValueError("V1 and EXTERNAL inventories have different native host or CPU pins")
    runner = next(Path(x.split("=", 1)[1]) for x in ordinary["argv"] if x.startswith("NATIVE_JOBS_ARTIFACTS="))
    broker = next(Path(x) for x in direct["verified_files"] if x.endswith("/broker"))
    producer_manifest = broker.with_name("mesh.json")
    if (protected["verified_files"][str(broker)] != direct["verified_files"][str(broker)]
            or protected["artifact_sha256"] != direct["artifact_sha256"]
            or direct["verified_files"][str(producer_manifest)] != direct["artifact_sha256"]
            or ordinary["verified_files"][str(runner / "mesh")] != v1_build["files"]["mesh"]
            or ordinary["verified_files"][str(runner / "mesh.json")] != ordinary["artifact_sha256"]
            or direct["version"] != v1_build["version"]
            or v1_build["execution_contract"] != "ordinary-trusted-shared-process"):
        raise ValueError("producer or V1 runtime differs from the recorded artifacts")
    runtime = Path(ordinary["argv"][-2]).parents[3] / "libmesh_rt.a"
    pg = f"{BASE}/toolchain/pgsql-17.11/bin/postgres"
    return {
        "runner_manifest_sha256": ordinary["artifact_sha256"],
        "runner_sha256": v1_build["files"]["mesh"],
        "broker_path": str(broker),
        "broker_version": direct["version"],
        "broker_source_sha256": "64f06243db4bb1b9fd2c04181c9faa46c207596b7e7f5686fd710d780ff11b22",
        "broker_sha256": direct["verified_files"][str(broker)],
        "broker_manifest_sha256": direct["artifact_sha256"],
        "runner_artifacts": str(runner),
        "dependencies": {
            str(runner / "mesh"): v1_build["files"]["mesh"],
            str(runner / "mesh.json"): ordinary["artifact_sha256"],
            str(runtime): v1_build["runtime_sha256"],
            str(broker): direct["verified_files"][str(broker)],
            str(producer_manifest): direct["artifact_sha256"],
            pg: protected["verified_files"][pg],
        },
    }


def amend_arm(path, replace_once):
    source = path.read_text()
    substitutions = (
        ('import ledger\n', 'import ledger\nimport assembly\n'),
        ('    path = PROOF / "trial.py"', '    path = HERE / "direct-trial.py"'),
        ('    def capture_runners(self, processes, logs):\n', assembly_subject()),
        ('    engine, profile = args.engine, args.profile\n', '''    engine, profile = args.engine, args.profile
    if (engine, profile) != ("mesh", "large"):
        raise ValueError("the composition is limited to the frozen large Mesh profile")
    pins, policy = assembly.configuration(HERE, artifacts, args.local_smoke)
'''),
        ('    build = json.loads(manifest_path.read_text())\n', '''    build = json.loads(manifest_path.read_text())
    assembly.verify_inputs(pins, artifacts, build)
'''),
        ('        subject = Subject(engine, [str(command)], artifacts, cluster, env)\n', '''        subject = Subject(engine, [str(command)], artifacts, cluster, env)
        subject.direct_producer = pins["broker_path"]
        subject.fixture_dir = scratch.resolve()
        subject.storage_policy = policy
'''),
        ('        database = database_settings(cluster, engine)\n', '''        for transaction in cluster.receipts:
            if subject.storage_setup["ddl_end_us"] > transaction["submitted_us"]:
                raise RuntimeError("storage setup crossed the frozen submit boundary")
        assembly.finish(cluster, subject, out, pins)
        database = database_settings(cluster, engine)
'''),
    )
    for old, new in substitutions:
        source = replace_once(source, old, new)
    compile(source, str(path), "exec")
    path.write_text(source)


def assembly_subject():
    return '''    def call(self, action, env, log):
        result = super().call(action, env, log)
        if action == "migrate":
            self.storage_setup = assembly.prepare_storage(self.cluster, self.storage_policy)
        return result

    def capture_runners(self, processes, logs):
        assembly.capture_processes(self, processes, h.process_table)
'''


def assemble(dest):
    if digest(V1) != V1_SHA:
        raise ValueError("V1 source archive changed")
    donor = LANE / "inputs/direct-prepare.py"
    if digest(donor) != DIRECT_PREPARE_SHA:
        raise ValueError("direct broker adapter source changed")
    spec = importlib.util.spec_from_file_location("retained_direct", donor)
    direct = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(direct)
    files = (
        *(f"{NATIVE}/{p}" for p in ("arm.py", "ledger.py", "run")),
        *(f"{TRIAL}/{p}" for p in ("trial.py", "oban_runtime.py", "proof_transport.py", "trusted-application.sql")),
        "scripts/jobs-parity.py", "scripts/jobs-parity-profiles.json", "scripts/lib/jobs_parity_fault.py",
    )
    dest.mkdir()
    with tarfile.open(V1) as archive:
        for name in files:
            path = dest / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(archive.extractfile(name).read())
    adapter = dest / NATIVE
    # Reuse the owner's exact direct-file trial transform. Keep V1 arm logic.
    stage = adapter.with_name("broker-transform")
    direct.prepare_adapter(dest, stage)
    trial = (stage / "direct-trial.py").read_text()
    trial = direct.replace_once(trial, '        output = evidence / "broker-output.log"',
                                 '''        env.pop("MESH_EFFECT_APP_URL", None)
        output = evidence / "broker-output.log"''')
    trial = direct.replace_once(trial, '        ) as broker, output.open("rb") as reader:\n',
                                '        ) as broker, output.open("rb") as reader:\n            self.broker_process = broker\n')
    (adapter / "direct-trial.py").write_text(trial)
    shutil.rmtree(stage)
    amend_arm(adapter / "arm.py", direct.replace_once)
    launcher = adapter / "run"
    launcher.write_text(direct.replace_once(launcher.read_text(),
                        '    "DURABLE_JOBS_PHASE=${DURABLE_JOBS_PHASE:-diagnostic}"',
                        '''    "DURABLE_JOBS_PHASE=${DURABLE_JOBS_PHASE:-diagnostic}"
    "ORDINARY_LARGE_STORAGE=${ORDINARY_LARGE_STORAGE:?select a frozen storage layout}"'''))
    launcher.chmod(0o755)
    shutil.copy2(Path(__file__).with_name("assembly.py"), adapter / "assembly.py")
    storage = (LANE / "inputs/storage-upstream.py").read_text()
    storage = direct.replace_once(storage, 'TABLES = {', '''TABLES = {
    "mesh": ("jobs", "args", "jsonb", "state", "completed"),''')
    storage = direct.replace_once(storage, '    expected = "apalis" if engine == "rust" else r"parity_[0-9a-f]{16}"',
                                  '''    expected = {"mesh": "mesh_jobs", "rust": "apalis"}.get(engine, r"parity_[0-9a-f]{16}")''')
    (adapter / "storage.py").write_text(storage)
    write(adapter / "assembly-pins.json", native_pins())


if __name__ == "__main__":
    import sys
    assemble(Path(sys.argv[1]).resolve())
