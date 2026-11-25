"""Verify the frozen adapter entry and enter its actual warm-producer context as nobody."""
import argparse
import hashlib
import inspect
import json
import os
from pathlib import Path
import runpy
import sys
from tempfile import TemporaryDirectory
import time
from types import SimpleNamespace

IDENTITY = dict(user="parity_producer", tls=True, durable=True, commit_timestamps=True,
                max_connections=96, database_connection_limit=96, reserve=0, superuser_reserve=0)


def sha(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


def exercise(scope, arm, temp, synthetic):
    evidence = temp / "evidence"
    scratch = temp / "db"
    evidence.mkdir()
    scratch.mkdir()
    artifact = Path(os.environ["NATIVE_JOBS_ARTIFACTS"])
    broker_path = Path(os.environ["DENSE_BROKER"])
    if synthetic:
        mock = temp / "broker"
        mock.write_text("#!/usr/local/bin/python3\nimport sys\n"
                        "assert sys.argv[1:] == ['session']\n"
                        "print('SESSION_READY', flush=True)\n"
                        "assert sys.stdin.readline() == 'stop\\n'\n"
                        "print('{}', flush=True)\n")
        mock.chmod(0o755)
        os.environ["DENSE_BROKER"] = str(mock)
        cluster = SimpleNamespace(env=os.environ.copy(), connection_cap=96, broker=None,
                                  url=lambda *_: "synthetic", sql=lambda *_: json.dumps(IDENTITY))
    else:
        cluster = scope["Cluster"](scratch, evidence, 96)
        cluster.trusted_application = True
        cluster.runners, cluster.slots, cluster.lanes = 2, 64, 16

    subject = scope["DenseSubject"]("mesh", [str(artifact / "mesh")], artifact, cluster, cluster.env)
    try:
        if not synthetic:
            cluster.start()
            cluster.prepare_database("mesh")
        env = subject.environment("parity_0123456789abcdef", 32, 64, "dense", 250)
        with subject.warm_producer(env, evidence, time.monotonic() + 15) as enqueue:
            assert callable(enqueue) and cluster.broker is not None
            broker, start = subject.broker_process
            assert broker.poll() is None
            if not synthetic:
                pin = os.environ["DENSE_BROKER_SHA256"]
                scope["observe"](broker, broker_path, "session", pin,
                                 scope["pinned_file"](broker_path, pin), start, [0, 1])
        assert cluster.broker is None and broker.returncode == 0
        assert json.loads((evidence / "database.json").read_text()) == IDENTITY
    finally:
        if not synthetic:
            cluster.stop()
            arm.redact(evidence, cluster.password)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("adapter", type=Path)
    parser.add_argument("--archived-asset-digests", type=Path)
    args = parser.parse_args()
    assert (os.getuid(), os.geteuid(), os.getgid()) == (65534, 65534, 65534)
    adapter = args.adapter.resolve(strict=True)
    assert str(adapter).startswith("/var/lib/durable-jobs-bench/native/source-dense-producer-1024-")
    assert sorted(os.sched_getaffinity(0)) == [0, 1]
    sys.path.insert(0, str(adapter.parent))
    scope = runpy.run_path(str(adapter))
    arm = scope["arm"]
    harness = Path(inspect.getsourcefile(arm.run_arm))
    if args.archived_asset_digests:
        pins = json.loads(args.archived_asset_digests.read_text())
        original_digest = arm.h.digest

        def archived_digest(path):
            if str(path) in pins:
                assert not Path(path).exists()
                return pins[str(path)]
            return original_digest(path)

        arm.h.digest = archived_digest

    def entered():
        active = arm.run_arm
        assert active.__code__.co_filename == str(harness)
        assert active.__globals__["capture_writer"] is scope["capture_writer"]
        assert active.__globals__["Subject"] is scope["DenseSubject"]
        assert active.__globals__["Cluster"] is scope["Cluster"]
        assert arm.PROFILES["noop"][:4] == (2, 64, 16, 96)
        with TemporaryDirectory(prefix="producer-context-", dir=os.environ["DENSE_PROBE_SCRATCH"]) as temp:
            exercise(scope, arm, Path(temp), bool(args.archived_asset_digests))
        print(json.dumps(dict(adapter=str(adapter), adapter_sha256=sha(adapter), harness=str(harness),
                              harness_sha256=sha(harness), uid=os.geteuid(), gid=os.getegid(), cpus=[0, 1],
                              width=os.environ["DENSE_WIDTH"], no_jobs_submitted=True,
                              context="synthetic-broker" if args.archived_asset_digests else "native-broker-tls"),
                         sort_keys=True))

    arm.main = entered
    sys.argv = [str(adapter), "mesh", "noop", "/unused"]
    scope["main"]()


if __name__ == "__main__":
    main()
