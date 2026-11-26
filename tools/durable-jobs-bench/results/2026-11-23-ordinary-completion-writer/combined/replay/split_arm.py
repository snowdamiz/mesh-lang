#!/usr/bin/env python3
"""Use the frozen bulk V2 arm and substitute only its Jobs runner subprocess."""
from collections import Counter
from copy import copy
import json
import os
from pathlib import Path
import sys

BULK_MANIFEST = "7f228c192630bb6d95cbf9e06c5e0132931f429c1cfa07391bb28cb6fdec8911"
BULK_ELF = "55d7000d562a6ad395f5e64e33e0e973fac0084ed05ad6c30a33db2a3fc6303a"
sys.path.insert(0, str(Path(os.environ["COMBINED_BULK_SOURCE"]) / "tools/durable-jobs-bench/native"))
import arm

BaseSubject = arm.Subject


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
            if self.runner_build["architecture"] != "x86_64":
                raise ValueError("expected Linux x86_64 runner")

    def run(self, env, index, log):
        runner = copy(self)
        runner.command = [str(self.runner_file)]
        runner.cwd = self.runner_dir
        env = dict(env)
        env.pop("TRUSTED_COMPLETION_WRITER_MS", None)
        if self.mode == "zero":
            env["TRUSTED_COMPLETION_WRITER_MS"] = "0"
        return BaseSubject.run(runner, env, index, log)

    def capture_runners(self, processes, logs):
        super().capture_runners(processes, logs)
        for process in processes:
            actual = Path(process.fixture_executable)
            if sys.platform == "linux":
                actual = Path(f"/proc/{process.pid}/exe").resolve(strict=True)
            if actual != self.runner_file:
                raise RuntimeError("observed a runner executable other than the sealed candidate")
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
    arm.Subject = SplitSubject
    arm.main()
