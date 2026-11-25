"""Untimed nonroot TLS lifecycle for the exact staged readiness code."""

from contextlib import contextmanager
from copy import deepcopy
import ctypes
import json
import os
from pathlib import Path
import runpy
import select
import subprocess
import sys

sys.path.insert(0, "/dense")
from control import connect, libpq

LIB = "libpq.so.5"
CERT = Path("/tls/server.crt")
PORT = 5432


def child(apps):
    assert os.geteuid() != 0
    assert ctypes.CDLL(None).prctl(4, 0, 0, 0, 0) == 0
    lib = libpq(LIB)
    from contextlib import ExitStack

    with ExitStack() as stack:
        def open_role(role):
            return stack.enter_context(connect(lib, PORT, CERT, role))

        before_replace = connect(lib, PORT, CERT, "compiled_caller")
        app = before_replace.__enter__()
        try:
            rest = [open_role("compiled_caller") for _ in range(apps - 1)]
            participants = [open_role("parity_subject") for _ in range(4)]
            print(json.dumps(dict(app=[lib.PQbackendPID(conn) for conn in (app, *rest)],
                                  participant=[lib.PQbackendPID(conn) for conn in participants])), flush=True)
            if sys.stdin.readline().strip() == "replace":
                before_replace.__exit__(None, None, None)
                before_replace = None
                print(lib.PQbackendPID(open_role("compiled_caller")), flush=True)
                sys.stdin.read()
        finally:
            if before_replace is not None:
                before_replace.__exit__(None, None, None)


def receive(process):
    if not select.select([process.stdout], [], [], 15)[0]:
        raise RuntimeError("TLS child did not report its live sockets")
    return json.loads(process.stdout.readline())


@contextmanager
def launch(apps):
    with subprocess.Popen([sys.executable, "-B", __file__, "child", str(apps)], stdin=subprocess.PIPE,
                          stdout=subprocess.PIPE, text=True) as process:
        try:
            yield process, receive(process)
        finally:
            process.stdin.close()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
            assert process.returncode == 0


def run(stage, out, runners):
    from contextlib import ExitStack
    from unittest.mock import patch

    assert os.geteuid() != 0 and sys.platform == "linux" and not os.access(stage, os.R_OK)
    slow = runpy.run_path(str(stage / "slow.py"))
    Cluster, Subject, identity, roster = (slow[name] for name in ("Cluster", "Subject", "identity", "roster"))
    slow["install"]()
    lib = libpq(LIB)
    assert lib.PQlibVersion() > 0
    with ExitStack() as stack:
        producer = stack.enter_context(connect(lib, PORT, CERT, "parity_producer"))
        cluster = object.__new__(Cluster)
        cluster.runners, cluster.evidence = runners, out

        def sql(_db, statement):
            result = lib.PQexec(producer, statement.encode())
            try:
                assert lib.PQresultStatus(result) == 2
                return lib.PQgetvalue(result, 0, 0).decode()
            finally:
                lib.PQclear(result)

        assert sql("mesh", "SHOW server_version_num") == "170011"
        cluster.sql = sql
        subject = object.__new__(Subject)
        subject.cluster = cluster
        # For the producer, the lifecycle control borrows the actual libpq
        # observer above. The stand-in child tests the independent process
        # start witness without introducing a second producer TLS session.
        broker = subprocess.Popen(["/bin/sleep", "40"])
        stack.callback(broker.wait, 5)
        stack.callback(broker.terminate)
        subject.broker_process = (broker, identity.process_stamp(broker))
        direct = []

        def test_launch(_subject, _env, _index, log):
            process, ids = stack.enter_context(launch(8 // runners))
            direct.append(ids)
            process.product_stamp = None, identity.process_stamp(process)
            log.write_text("READY\n")
            return process

        with patch.object(slow["public"].Subject, "run", test_launch):
            processes = [subject.run({}, index, out / f"runner-{index}.log")
                         for index in range(runners)]
        cluster.audit_tls("mesh")
        original_map = runpy.run_path("/original-stage/socket_map.py")["runner_backend_map"]
        for process in processes:
            try:
                original_map([process.pid], PORT, roster.standard(cluster.ready_roster))
            except PermissionError as exc:
                assert exc.errno == 13 and exc.filename == f"/proc/{process.pid}/fd"
            else:
                raise AssertionError("nondumpable child unexpectedly exposed its FD directory")
        subject.verify_before_submit(processes)
        witness = json.loads((out / "ready-roster.json").read_text())
        ready = roster.snapshot(witness["ready"], runners, runners)
        direct_all = {lib.PQbackendPID(producer)} | {
            n for ids in direct for by_role in ids.values() for n in by_role}
        assert ready.keys() == direct_all
        for process, ids in zip(processes, direct, strict=True):
            assert {row["backend"] for row in cluster.warm_apps_by_runner[str(process.pid)]} == set(ids["app"])
            assert all(ready[backend][0] == "parity_subject" for backend in ids["participant"])

        refusals = {}
        def refuse(name, fun):
            try:
                fun()
            except ValueError as exc:
                refusals[name] = str(exc)
            else:
                raise AssertionError(f"accepted invalid roster: {name}")

        pids = [p.pid for p in processes]
        for name, mutate in (
            ("short", lambda f: f["sessions"].pop()),
            ("unexpected_role", lambda f: f["sessions"][0].update(role="unowned")),
            ("lost_tls", lambda f: f["sessions"][0].update(tls=False)),
            ("changed_port", lambda f: f["sessions"][0].update(client_port=1)),
            ("reused_port", lambda f: f["sessions"][0].update(client_port=f["sessions"][1]["client_port"])),
            ("changed_start", lambda f: f["sessions"][0].update(backend_start_us=1)),
            ("changed_cap", lambda f: f["caps"].update(max_connections=71)),
        ):
            changed = deepcopy(witness)
            mutate(changed["before_submit"])
            refuse(name, lambda: roster.verify(changed, pids))
        refuse("wrong_pid_order", lambda: roster.verify(witness, [*pids[:-1], pids[-1] + 1]))
        processes[0].stdin.write("replace\n")
        processes[0].stdin.flush()
        replacement = receive(processes[0])
        assert replacement not in direct_all
        fresh = cluster.warm_roster(runners)
        assert set(roster.snapshot(fresh, runners, runners)) == (direct_all - {direct[0]["app"][0]}) | {replacement}
        refuse("live_same_count_replacement", lambda: subject.verify_before_submit(processes))

    print(json.dumps(dict(uid=os.geteuid(), kernel=os.uname().sysname, architecture=os.uname().machine,
                          postgres_version=170011, runners=runners, slots=64, stage_listable=False,
                          fd_denials=runners, roster_sizes=[len(frame["sessions"]) for frame in
                              (*witness["prefixes"], witness["ready"], witness["before_submit"])],
                          libpq_owner_counts=[len(group["app"]) for group in direct],
                          caps=witness["ready"]["caps"], refusals=refusals), sort_keys=True))


if __name__ == "__main__":
    if sys.argv[1] == "child":
        child(int(sys.argv[2]))
    else:
        run(Path(sys.argv[1]), Path(sys.argv[2]), int(sys.argv[3]))
