#!/usr/bin/env python3
"""Use the frozen bulk V2 arm and substitute only its Jobs runner subprocess."""
from collections import Counter
from contextlib import contextmanager
from copy import copy
import inspect
import json
import os
from pathlib import Path
import sys
from textwrap import dedent

from identity import observe, pinned_file, process_stamp

BULK_MANIFEST = "7f228c192630bb6d95cbf9e06c5e0132931f429c1cfa07391bb28cb6fdec8911"
BULK_ELF = "55d7000d562a6ad395f5e64e33e0e973fac0084ed05ad6c30a33db2a3fc6303a"
sys.path.insert(0, str(Path(os.environ["COMBINED_BULK_SOURCE"]) / "tools/durable-jobs-bench/native"))
import arm

BaseSubject = arm.Subject
TRIAL_SHA256 = "db8e121ab9b1f6aed2293b26c79a4fbfa8ac8aba6ff228be54b360bd32048a8e"
RUNNER_MANIFEST = "361f10d2c2866f694989198bb8359d29c91dd9928dcd95e8f92293293f226d29"
RUNNER_ELF = "93fef96704ae412cf3c55fae73cc2aac405c8b423670826b4876954a44c8b010"


def install_broker_observer():
    # The pinned regression adapter holds the direct Popen in its context.
    # Expose that handle at spawn while keeping the broker protocol and cleanup.
    original = arm.regression.Subject.warm_producer.__wrapped__
    path = Path(original.__code__.co_filename)
    if arm.h.digest(path) != TRIAL_SHA256:
        raise RuntimeError("frozen broker launch source changed")
    source = dedent(inspect.getsource(original))
    anchor = '    ) as broker, output.open("rb") as reader:\n'
    if source.count(anchor) != 1:
        raise RuntimeError("frozen direct broker launch anchor changed")
    source = source.replace(anchor, anchor + "        self.observe_broker_launch(broker)\n")
    scope = {}
    exec(compile(source, path, "exec"), original.__globals__, scope)
    arm.regression.Subject.warm_producer = scope["warm_producer"]


class SplitSubject(BaseSubject):
    def __init__(self, engine, command, cwd, cluster, env, **options):
        super().__init__(engine, command, cwd, cluster, env, **options)
        self.broker = Path(command[0]).resolve(strict=True)
        self.runner_dir = Path(os.environ["COMBINED_RUNNER_ARTIFACTS"]).resolve(strict=True)
        self.runner_file = (self.runner_dir / "mesh").resolve(strict=True)
        self.mode = os.environ["COMBINED_WRITER_MODE"]
        if self.mode not in {"off", "zero"} or self.broker == self.runner_file:
            raise ValueError("expected a separate checked runner and the off or zero switch")

        broker_manifest = self.broker.with_name("mesh.json")
        runner_manifest = self.runner_dir / "mesh.json"
        self.runner_build = json.loads(runner_manifest.read_text())
        if self.runner_build.get("capsule") != "bulk-v2-completion-writer-exit-r2":
            raise ValueError("expected the exit-confirmed runner artifact")
        self.broker_manifest_sha = arm.h.digest(broker_manifest)
        self.broker_sha = arm.h.digest(self.broker)
        self.runner_sha = arm.h.digest(self.runner_file)
        self.runner_manifest_sha = arm.h.digest(runner_manifest)
        if self.runner_sha != self.runner_build["files"]["mesh"]:
            raise ValueError("runner executable failed its seal")
        if self.runner_build["system"] == "Linux":
            if (self.broker_manifest_sha, self.broker_sha) != (BULK_MANIFEST, BULK_ELF):
                raise ValueError("native producer and migrator must use exact prior bulk V2 files")
            if (self.runner_manifest_sha, self.runner_sha) != (RUNNER_MANIFEST, RUNNER_ELF):
                raise ValueError("native runner and manifest must match the retained r2 build")
            if self.command != [str(self.broker)]:
                raise ValueError("expected the direct bulk broker command")
            if self.runner_build["architecture"] != "x86_64":
                raise ValueError("expected Linux x86_64 runner")

        self.runner_launches = {}
        self.broker_launch = None

    @contextmanager
    def warm_producer(self, env, evidence, deadline):
        if sys.platform == "linux":
            self.broker_file_stamp = pinned_file(self.broker, self.broker_sha)
        try:
            with super().warm_producer(env, evidence, deadline) as enqueue:
                yield enqueue
        finally:
            self.broker_launch = None

    def observe_broker_launch(self, process):
        if self.broker_launch is not None:
            raise RuntimeError("unexpected second direct broker")
        self.broker_launch = (process, process_stamp(process))

    def run(self, env, index, log):
        runner = copy(self)
        runner.command = [str(self.runner_file)]
        runner.cwd = self.runner_dir
        env = dict(env)
        env.pop("TRUSTED_COMPLETION_WRITER_MS", None)
        if self.mode == "zero":
            env["TRUSTED_COMPLETION_WRITER_MS"] = "0"
        if sys.platform != "linux":
            return BaseSubject.run(runner, env, index, log)
        executable_stamp = pinned_file(self.runner_file, self.runner_sha)
        process = BaseSubject.run(runner, env, index, log)
        try:
            self.runner_launches[process] = (executable_stamp, process_stamp(process))
        except BaseException:
            arm.h.stop_runners([process], require_clean=False)
            raise
        return process

    def capture_runners(self, processes, logs):
        super().capture_runners(processes, logs)
        if len(processes) != 1 or any(Path(process.fixture_executable) != self.runner_file for process in processes):
            raise RuntimeError("expected the one direct short-profile runner")
        if sys.platform == "linux":
            process, = processes
            if self.broker_launch is None or set(self.runner_launches) != {process}:
                raise RuntimeError("missing directly launched broker or runner handle")
            cpus = sorted(os.sched_getaffinity(0))
            broker, broker_start = self.broker_launch
            if broker.pid == process.pid:
                raise RuntimeError("broker and runner must be separate processes")
            runner_file_stamp, runner_start = self.runner_launches[process]
            launches = dict(
                broker=observe(broker, self.broker, "session", self.broker_sha,
                               self.broker_file_stamp, broker_start, cpus),
                runner=observe(process, self.runner_file, "run", self.runner_sha,
                               runner_file_stamp, runner_start, cpus))
            arm.h.write_json(self.cluster.evidence / "process-identity.json", dict(
                adapter_pid=os.getpid(), cpus=cpus, launches=launches))
        tls = json.loads((self.cluster.evidence / "postgres-tls-sessions.json").read_text())
        roles = Counter(row["role"] for row in tls)
        if roles["compiled_caller"] != 8 or roles["parity_producer"] != 1 or not 2 <= roles["parity_subject"] <= 4:
            raise RuntimeError(f"runner, warm pool or producer physical connection layout changed: {roles}")
        arm.h.write_json(self.cluster.evidence / "split-executables.json", dict(
            broker_sha256=self.broker_sha, broker_manifest_sha256=self.broker_manifest_sha,
            producer=str(self.broker), migrator=str(self.broker), runner=str(self.runner_file),
            runner_sha256=self.runner_sha, runner_manifest_sha256=self.runner_manifest_sha,
            runner_version=self.runner_build["version"], writer_mode=self.mode,
            runner_pids=[process.pid for process in processes], warmed_application_sessions=8,
        ))


if __name__ == "__main__":
    if sys.platform == "linux":
        install_broker_observer()
    arm.Subject = SplitSubject
    arm.main()
