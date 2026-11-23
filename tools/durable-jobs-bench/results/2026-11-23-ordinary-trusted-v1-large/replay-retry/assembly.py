"""Check the direct broker and record disposable payload storage around the trial."""

import hashlib
import json
import os
from pathlib import Path
import sys
import time

import storage


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def configuration(here, artifacts, local_smoke):
    pins_file = Path(os.environ["ORDINARY_LARGE_LOCAL_PINS"]) if local_smoke else here / "assembly-pins.json"
    pins = json.loads(pins_file.read_text())
    policy = os.environ["ORDINARY_LARGE_STORAGE"]
    if policy not in {"external", "default"} or Path(pins["runner_artifacts"]) != artifacts:
        raise ValueError("unknown storage label or unexpected V1 artifact path")
    return pins, policy


def verify_inputs(pins, artifacts, build):
    broker = Path(pins["broker_path"])
    manifest = broker.with_name("mesh.json")
    paths = {
        broker: pins["broker_sha256"],
        manifest: pins["broker_manifest_sha256"],
        artifacts / "mesh.json": pins["runner_manifest_sha256"],
        artifacts / "mesh": pins["runner_sha256"],
    }
    for path, checksum in paths.items():
        if digest(path) != checksum:
            raise RuntimeError(f"assembly input changed: {path.name}")
    producer = json.loads(manifest.read_text())
    if (producer.get("producer") != "mesh-direct-file"
            or producer["files"]["broker"] != pins["broker_sha256"]
            or producer["version"] != pins["broker_version"]
            or producer["files"]["broker-source/main.mpl"] != pins["broker_source_sha256"]
            or build["files"]["mesh"] != pins["runner_sha256"]):
        raise RuntimeError("wrong direct-file broker or ordinary V1 build")


def capture_processes(subject, processes, process_table):
    pids = [process.pid for process in processes]
    broker = subject.broker_process
    if broker.pid in pids or len(set(pids)) != subject.cluster.runners:
        raise RuntimeError("unexpected direct broker and ordinary V1 process topology")
    masks, launches = {}, {}
    if sys.platform == "linux":
        expected = sorted(os.sched_getaffinity(0))
        masks = {pid: sorted(os.sched_getaffinity(pid)) for pid in (*pids, broker.pid)}
        if any(mask != expected for mask in masks.values()):
            raise RuntimeError("broker or runner CPU affinity differs from the trial")
        adapter_pid = os.getpid()
        before = process_table()
        children = [(broker, [subject.direct_producer, "session"]),
                    *((process, [subject.command[0], "run"]) for process in processes)]
        for process, argv in children:
            pid = process.pid
            if process.poll() is not None or process.args != argv or before.get(pid, (None,))[0] != adapter_pid:
                raise RuntimeError(f"{pid}: expected a live, directly launched child")
            proc = Path(f"/proc/{pid}")
            if (proc / "cmdline").read_bytes() != b"".join(os.fsencode(arg) + b"\0" for arg in argv):
                raise RuntimeError(f"{pid}: live command line differs from the pinned launch")
            try:
                executable = os.readlink(proc / "exe")
            except PermissionError:
                # Optimized Mesh binaries disable dumpability. The pin and owned
                # Popen launch still identify what this harness executed.
                executable = None
            if executable is not None and executable != str(Path(argv[0]).resolve(strict=True)):
                raise RuntimeError(f"{pid}: kernel executable differs from the pinned launch")
            launches[pid] = dict(argv=argv, start_ticks=int(before[pid][2]), parent_pid=adapter_pid,
                                 method="owned-launch" if executable is None else "proc-exe",
                                 observed_executable=executable)
        after = process_table()
        if any(process.poll() is not None or pid not in after or
               (after[pid][0], int(after[pid][2])) != (adapter_pid, launches[pid]["start_ticks"])
               for process, _ in children for pid in (process.pid,)):
            raise RuntimeError("an owned child disappeared or changed during identity capture")
    subject.assembly_processes = dict(broker_pid=broker.pid, runner_pids=pids, affinity=masks,
                                      adapter_pid=os.getpid(), launches=launches)


def prepare_storage(cluster, policy):
    if policy == "external":
        return storage.apply(cluster, "mesh", "mesh", "mesh_jobs")
    relation = "mesh_jobs.jobs"
    stamp = time.time_ns() // 1_000
    observed = json.loads(cluster.sql("mesh", storage.metadata(relation, "args")))
    rows = int(cluster.sql("mesh", f"SELECT count(*) FROM {relation}"))
    if rows or observed["attstorage"] != "x" or observed["type"] != "jsonb":
        raise RuntimeError("expected empty EXTENDED Mesh payload column")
    return dict(before=observed, after=observed, rows_before=rows, ddl=None,
                ddl_start_us=stamp, ddl_end_us=time.time_ns() // 1_000)


def finish(cluster, subject, out, pins):
    ledger = cluster.jobs
    setup = subject.storage_setup
    started = time.time_ns() // 1_000
    persisted = json.loads(cluster.sql("mesh", storage.snapshot("mesh", "mesh_jobs.jobs")))
    finished = time.time_ns() // 1_000
    expected = sorted((row["id"], row["job_key"], row["payload_sha256"], row["settled_xid"]) for row in ledger)
    actual = sorted((row["id"], row["job_key"], row["payload_sha256"], row["settled_xid"]) for row in persisted["rows"])
    external = subject.storage_policy == "external"
    if (len(expected) != 64 or actual != expected or persisted["metadata"] != setup["after"]
            or not persisted["trigger_absent"] or started < max(job["settled_us"] for job in ledger)
            or any(row["payload_bytes"] != 512 * 1024 or row["native_state"] != "completed"
                   or row["stored_column_bytes"] <= 0 or (external and row["compression"] is not None)
                   for row in persisted["rows"])):
        raise RuntimeError("stored values differ from the committed V1 job ledger")
    terminal = {row["settled_xid"] for row in ledger}
    work = {row[field] for row in ledger for field in ("attempt_xid", "effect_xid")}
    if len(terminal) != 64 or len(work) != 128 or terminal & work:
        raise RuntimeError("V1 requires independent application and individual terminal commits")
    storage.write(out / "payload-storage.json", dict(
        engine="mesh", policy="EXTERNAL" if external else "EXTENDED", setup=setup, persisted=persisted,
        verify_start_us=started, verify_end_us=finished))
    storage.write(out / "assembly.json", dict(
        execution_contract="ordinary-trusted-shared-process", producer="mesh-direct-file",
        runner_sha256=pins["runner_sha256"], broker_sha256=pins["broker_sha256"],
        storage_policy=subject.storage_policy, **subject.assembly_processes))
