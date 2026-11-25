"""Exercise the captured controller wrapper and owner's actual timing entry offline."""

import ast
from collections import Counter
from contextlib import chdir, redirect_stdout
from datetime import datetime, timedelta, timezone
import importlib.util
import json
import io
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
from types import SimpleNamespace

HERE = Path(__file__).resolve().parent
OWNER = HERE.parents[1]
WRAPPER = HERE / "wrapper.py.txt"
RELEASE = OWNER / "evidence/native/controller-release.json"
STDERR = OWNER / "evidence/native/controller-wrapper-stderr.txt"
PREDECESSOR = (OWNER.parents[1] / "2026-11-25-dense-same-page/wal-retention-retry"
               / "evidence/native/controller-release.json")


class FirstDispatch(Exception):
    """Stop without starting an arm."""


def parsed(path):
    return ast.parse(path.read_text(), filename=str(path))


def load_entry():
    sys.path.insert(0, str(OWNER))
    spec = importlib.util.spec_from_file_location("offline_owner", OWNER / "controller.py")
    owner = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(owner)
    return owner


def main():
    # The only effects allowed below are reads and writes inside the scratch
    # tree and Python execution in this process. Reject every real child and
    # socket connection, including any path that escaped the explicit stubs.
    def offline_only(event, args):
        if event in {"subprocess.Popen", "socket.connect", "socket.getaddrinfo"}:
            raise RuntimeError(f"offline entry attempted {event}")

    sys.addaudithook(offline_only)
    owner = load_entry()
    run = owner.run
    packet = run.read_json(owner.PACKET)
    release = run.read_json(RELEASE)
    assert release["exit_status"] == 1 and release["request_sha256"] == run.digest(owner.PACKET)
    assert release["released_at"].startswith("2026-11-25T02:00:58.")
    assert STDERR.read_text().endswith("AssertionError\n")

    wrapper_tree = parsed(WRAPPER)
    queued = [call for call in ast.walk(wrapper_tree.body[-1]) if isinstance(call, ast.Call)
              and isinstance(call.func, ast.Name) and call.func.id == "slot"]
    relative, seal, predecessor, groups = [ast.literal_eval(arg) for arg in queued[0].args]
    assert seal == release["request_sha256"] and groups == release["groups"] == [1, 2]
    assert ast.literal_eval(queued[1].args[2]) == release["id"]
    predecessor_release = run.read_json(PREDECESSOR)
    assert predecessor_release["exit_status"] == 0 and predecessor_release["released_at"] < release["started"]
    assert predecessor == predecessor_release["id"] + "-groups-1-2"

    actual_entry = parsed(OWNER / "controller.py").body[-1]
    child_main = compile(ast.Module(body=[actual_entry], type_ignores=[]),
                         str(OWNER / "controller.py"), "exec")
    staged, probes, preflights, boundary = [], [], [], []

    with tempfile.TemporaryDirectory(prefix="dense-entry-offline-") as scratch:
        temp = Path(scratch)
        queue = temp / "requests"
        queue.mkdir()
        journal = queue / "STATUS.md"
        journal.write_text(f"RELEASED {predecessor} {predecessor_release['released_at']} exit 0\n")
        shutil.copyfile(owner.PACKET, queue / f"{packet['id']}.json")
        shutil.copyfile(PREDECESSOR, queue / f"{predecessor}.released.json")
        suite = temp / "suite"
        suite.mkdir()
        (suite / ".timing.lock").touch()
        shutil.copyfile(owner.base.SUITE / "worker.py", suite / "worker.py")
        owner.base.SUITE, owner.base.QUEUE, owner.OUT = suite, queue, temp / "native"

        sealed_cwd, argv = shlex.split(packet["timing_command"])[1], shlex.split(packet["timing_command"])[3:]

        def fixture_path(value):
            if value == packet["evidence_destination"]:
                return owner.OUT
            if value == sealed_cwd:
                return OWNER
            return Path(value)

        def note(value):
            with journal.open("a") as output:
                output.write(value + "\n")

        def preflight(args, **options):
            assert args[:2] == ["bash", "-c"] and len(args) == 3
            policy = args[2]
            assert all(pin in policy for pin in (*packet["native_host"], packet["module_sha256"],
                                                 packet["original_module"], "runuser -u nobody", "flock -n 8",
                                                 "flock -n 9", "taskset -c 0,1"))
            preflights.append(options["timeout"])
            return subprocess.CompletedProcess(args, 0)

        def stage(path, remote, log):
            assert path.is_file() and log.writable()
            staged.append((str(remote), path.name, run.digest(path)))

        def dispatch(folder, remote, block, phase, engine):
            assert folder.is_dir() and phase == "measured" and engine == "mesh"
            assert block == packet["schedule"][0]
            assert str(remote).endswith(f"/{packet['id']}-off-a")
            inventory = run.read_json(owner.OUT / "staging/off-a/inventory.json")
            pins = inventory["engines"]["mesh"]["verified_files"]
            assert all(pins[f"{remote}/{name}"] == digest for stage_dir, name, digest in staged
                       if stage_dir == str(remote) and name != "inventory.json")
            boundary.append(dict(group=block["group"], phase=phase, lane="off-a", engine=engine))
            raise FirstDispatch

        def child(command, *, cwd, stdout, stderr):
            assert command == argv == ["python3", "-B", "controller.py", "timing"]
            assert cwd == OWNER and stdout is stderr and stdout.writable()
            assert not owner.OUT.exists()
            owner.run.ssh, owner.run.stage, owner.run.dispatch_arm = preflight, stage, dispatch
            old_argv = sys.argv
            sys.argv = command[2:]
            try:
                with chdir(cwd):
                    owner.__dict__["__name__"] = "__main__"
                    exec(child_main, owner.__dict__)
            except FirstDispatch:
                return subprocess.CompletedProcess(command, 42)
            finally:
                sys.argv = old_argv
            raise AssertionError("entry returned before the dispatch stub")

        scope = dict(Path=fixture_path, RUN=run, QUEUE=queue, STATE=temp,
                     OPS={"note": note, "probe": lambda: probes.append("offline_stub")},
                     datetime=datetime, timedelta=timedelta, timezone=timezone,
                     shlex=shlex, re=re, subprocess=SimpleNamespace(run=child, SubprocessError=subprocess.SubprocessError))
        functions = [node for node in wrapper_tree.body if isinstance(node, ast.FunctionDef)]
        exec(compile(ast.Module(body=functions, type_ignores=[]), str(WRAPPER), "exec"), scope)
        assert scope["command"](packet, "timing_command", OWNER) == (OWNER, argv)
        try:
            scope["command"]({**packet, "timing_command": "cd / && python3 -B controller.py timing"},
                             "timing_command", OWNER)
        except AssertionError:
            pass
        else:
            raise AssertionError("wrapper accepted an unrelated working directory")
        with redirect_stdout(io.StringIO()):
            scope["slot"](relative, seal, predecessor, groups)

        assert run.read_json(owner.OUT / "controller-release.json")["exit_status"] == 42
        assert len(probes) == 2 and preflights == [12] and len(boundary) == 1
        assert sorted(Counter(remote for remote, _, _ in staged).values()) == [9, 9, 9]

    print(json.dumps(dict(
        kind="offline_stubs_only", packet_sha256=seal, wrapper_sha256=run.digest(WRAPPER),
        source_stderr_sha256=run.digest(STDERR), native_failure_release_sha256=run.digest(RELEASE),
        failed_wrapper_cwd=sealed_cwd.rsplit("/tools/", 1)[0],
        sealed_child_working_directory=sealed_cwd, child_argument_vector=argv,
        failure="wrapper root-only assertion before child spawn; native child log empty",
        predecessor=predecessor, groups=groups, staged_files=len(staged),
        stubbed_preflight=len(preflights), first_dispatch=boundary[0],
        live_hosts_accessed=0, live_arms_started=0,
    ), indent=2))


if __name__ == "__main__":
    main()
