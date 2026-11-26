"""Exercise the pinned arm's direct launch, readiness and rejection in a Linux guest."""
from hashlib import file_digest
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import time
from types import SimpleNamespace

import identity
from identity import observe, pinned_file, process_stamp
import split_arm as split

FIXTURES = Path("/writer-controls")
TMP = Path("/tmp/writer-controls")
CPUS = sorted(os.sched_getaffinity(0))


def digest(path):
    with path.open("rb") as stream:
        return file_digest(stream, "sha256").hexdigest()


def refuse(check, message):
    try:
        check()
    except RuntimeError as error:
        if message in str(error):
            return
        raise
    raise RuntimeError(f"control was accepted: {message}")


def readiness(mode, queue_sessions):
    folder = TMP / f"{mode}-{queue_sessions}"
    folder.mkdir()
    runner = FIXTURES / "runner"
    broker = FIXTURES / "broker"
    log = folder / "runner.log"
    mocked_tls = [dict(role=role, tls=True) for role in
                  ["compiled_caller"] * 8 + ["parity_producer"] + ["parity_subject"] * queue_sessions]
    pg = folder / "pg"
    pg.mkdir()
    (pg / "postmaster.pid").write_text(str(os.getpid()) + "\n")
    expected_db = dict(user="parity_producer", tls=True, durable=True, commit_timestamps=True,
                       max_connections=20, database_connection_limit=20, reserve=0, superuser_reserve=0)
    def audit_tls(_engine):
        split.arm.h.write_json(folder / "postgres-tls-sessions.json", mocked_tls)

    cluster = SimpleNamespace(evidence=folder, data=pg, connection_cap=20, runners=1,
                              trusted_application=True, audit_tls=audit_tls,
                              sql=lambda *_args: json.dumps(expected_db), url=lambda *_args: "fixture")
    # Supply only manifest metadata here. Verify live file bytes and launch paths via the real methods.
    subject = split.SplitSubject.__new__(split.SplitSubject)
    subject.__dict__.update(
        engine="mesh", command=[str(broker)], cwd=FIXTURES, cluster=cluster, mode=mode,
        broker=broker, broker_sha=digest(broker), broker_manifest_sha="fixture-broker",
        runner_file=runner, runner_dir=FIXTURES, runner_sha=digest(runner),
        runner_manifest_sha="fixture-runner", runner_build={"version": "fixture"},
        runner_launches={}, broker_launch=None)
    env = dict(os.environ, DUMPABLE="0")
    with subject.warm_producer(env, folder, time.monotonic() + 10):
        process = subject.run(env, 0, log)
        try:
            split.arm.h.wait_for(lambda: split.arm.h.runner_ready([process], [log]),
                                 time.monotonic() + 5, "real fixture readiness")
            if queue_sessions == 5:
                refuse(lambda: subject.capture_runners([process], [log]), "connection layout changed")
                return {"mode": mode, "queue_sessions": queue_sessions, "result": "refused"}
            subject.capture_runners([process], [log])
            result = json.loads((folder / "process-identity.json").read_text())
            assert all(row["method"] == "owned-launch" and row["exe_errno"] in {1, 13}
                       and row["observed_executable"] is None for row in result["launches"].values())
            return dict(result, mode=mode, queue_sessions=queue_sessions)
        finally:
            split.arm.h.stop_runners([process], require_clean=False)


def refusal_checks():
    executable = FIXTURES / "runner"
    expected = digest(executable)
    stamp = pinned_file(executable, expected)
    env = dict(os.environ, DUMPABLE="1")
    argv = [str(executable), "run"]
    def launch(args=argv, **kwargs):
        process = subprocess.Popen(args, stdout=subprocess.PIPE, env=env, text=True, **kwargs)
        assert process.stdout.readline().strip() == "READY"
        return process

    checks = []
    def control(name, fn, message):
        refuse(fn, message)
        checks.append(name)

    process = launch()
    launch_stamp = process_stamp(process)
    def capture(file_stamp=stamp, start=launch_stamp):
        return observe(process, executable, "run", expected, file_stamp, start, CPUS)

    try:
        assert capture()["method"] == "proc-exe"
        control("digest-substitution", lambda: pinned_file(executable, "0" * 64), "digest mismatch")
        control("inode-substitution", lambda: capture(file_stamp=pinned_file(FIXTURES / "broker", expected)),
                "replaced since launch")
        control("launch-stamp-reuse", lambda: capture(start=(launch_stamp[0], launch_stamp[1] + 1)),
                "owned launch identity changed")
        process.args = [str(executable), "wrong-verb"]
        control("popen-argv", capture, "owned launch identity changed")
        process.args = argv
        os.sched_setaffinity(process.pid, {CPUS[0]})
        control("cpu-drift", capture, "CPU affinity differs")
        os.sched_setaffinity(process.pid, CPUS)
        # Force a changed second observation while retaining a real process and first read.
        original = identity.process_stamp
        reads = iter((original(process), (launch_stamp[0], launch_stamp[1] + 1)))
        identity.process_stamp = lambda _process: next(reads)
        try:
            control("start-tick-sandwich", capture, "changed during readiness")
        finally:
            identity.process_stamp = original
    finally:
        process.terminate()
        process.wait(timeout=5)
    control("dead-child", capture, "no longer alive")

    process = launch([str(executable), "other"])
    try:
        supplied = process_stamp(process)
        process.args = argv
        control("kernel-argv", lambda: observe(process, executable, "run", expected, stamp, supplied, CPUS),
                "live argv differs")
    finally:
        process.terminate()
        process.wait(timeout=5)

    process = launch(executable=FIXTURES / "broker")
    try:
        supplied = process_stamp(process)
        control("readable-executable-substitution",
                lambda: observe(process, executable, "run", expected, stamp, supplied, CPUS),
                "kernel executable differs")
    finally:
        process.terminate()
        process.wait(timeout=5)

    writable = TMP / "writable"
    shutil.copyfile(executable, writable)
    control("benchmark-writable-image", lambda: pinned_file(writable, expected), "can replace executable")
    with subprocess.Popen([str(executable), "run"], stdout=subprocess.PIPE, env=env, text=True) as child:
        assert child.stdout.readline().strip() == "READY"
        code = ("import sys; from types import SimpleNamespace; from identity import process_stamp;"
                " process_stamp(SimpleNamespace(pid=int(sys.argv[1]), poll=lambda: None))")
        result = subprocess.run([os.sys.executable, "-B", "-c", code, str(child.pid)],
                                cwd=FIXTURES, capture_output=True, text=True)
        child.terminate()
        assert result.returncode == 1 and "expected a live direct child" in result.stderr
        checks.append("wrong-parent-real-proc-stat")
    return checks


if __name__ == "__main__":
    assert os.geteuid() == 65534 and os.getegid() == 65534
    assert platform.system() == "Linux" and len(CPUS) == 2
    assert Path("/proc/self/status").read_text().split("CapEff:\t")[1].splitlines()[0] == "0000000000000000"
    TMP.mkdir()
    split.install_broker_observer()
    results = [readiness(mode, sessions) for mode, sessions in
               (("off", 2), ("zero", 3), ("off", 4), ("zero", 5))]
    print("WRITER_LINUX_CONTROL " + json.dumps(dict(
        kernel=platform.release(), architecture=platform.machine(), uid=os.geteuid(),
        effective_capabilities=0, readiness=results, refusals=refusal_checks()), sort_keys=True), flush=True)
