#!/usr/bin/env python3
"""Build the guide example and exercise it against a disposable PostgreSQL server."""

import argparse
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
NAMESPACE = "tenant-a"
TRUSTED = "trusted_shared_process"


def run(command, *, env=None, input=None, check=True):
    proc = subprocess.run(
        command, env=env, input=input, text=True, capture_output=True, timeout=90
    )
    if check and proc.returncode:
        raise RuntimeError(f"{Path(command[0]).name} failed\n{proc.stdout}{proc.stderr}")
    return proc


def wait(label, probe):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if result := probe():
            return result
        time.sleep(.05)
    raise AssertionError(f"timed out waiting for {label}")


def config(**options):
    return json.dumps(dict(
        namespace=NAMESPACE, queues={"default": 1},
        workers=[dict(worker="record-receipt", version=1)],
        lease_ms=900, poll_ms=25, shutdown_grace_ms=0, **options,
    ))


def exercise(directory, binary, owner_url):
    app_url = owner_url.replace("proof_owner@", "tenant_receipts@")
    env = dict(os.environ, QUEUE_DATABASE_URL=owner_url,
               TENANT_APP_DATABASE_URL=app_url, MESH_SCHEDULER_MIN_WORKERS="2",
               MESH_SCHEDULER_MAX_WORKERS="2")
    psql = ["psql", "-XqAt", "-v", "ON_ERROR_STOP=1", owner_url]

    def sql(statement):
        return run(psql, input=statement).stdout.strip()

    def app(action, spec=None, runner_config=None, **options):
        return run([str(binary)], env=dict(
            env, ACTION=action, SPEC=json.dumps(spec), RUNNER_CONFIG=runner_config or "",
        ), **options)

    def enqueue(key, timeout_ms=8000):
        app("enqueue", dict(worker="record-receipt", args=dict(receipt_key=key),
                            timeout_ms=timeout_ms, max_attempts=3))
        return wait(key, lambda: job(key))

    def job(key):
        row = sql(f"""SELECT json_build_array(id, state, attempt, errors)
            FROM mesh_jobs.jobs WHERE namespace = '{NAMESPACE}' AND
                args->>'receipt_key' = '{key}'""")
        return json.loads(row) if row else None

    def reaches(key, state):
        return wait(f"{key} {state}", lambda: row if (row := job(key))[1] == state else None)

    def effects(key):
        return int(sql(f"SELECT count(*) FROM tenant_app.receipts WHERE receipt_key = '{key}'"))

    children = []

    def start(runner_config):
        name = f"runner-{len(children)}"
        stop, stdout, stderr = (directory / f"{name}.{ext}" for ext in ("stop", "out", "err"))
        with stdout.open("wb") as out, stderr.open("wb") as err:
            child = subprocess.Popen([str(binary)], stdout=out, stderr=err, env=dict(
                env, ACTION="serve", RUNNER_CONFIG=runner_config, STOP_FILE=str(stop),
            ))
        children.append(child)

        def ready():
            if child.poll() is not None:
                raise AssertionError(stdout.read_text() + stderr.read_text())
            return "ready\n" in stdout.read_text()

        wait("runner ready", ready)
        return child, stop, stdout, stderr

    def line(path, prefix):
        return next((row[len(prefix):] for row in path.read_text().splitlines()
                     if row.startswith(prefix)), None)

    def stop_runner(runner):
        child, stop, stdout, stderr = runner
        stop.touch()
        assert child.wait(timeout=30) == 0, stderr.read_text()
        assert line(stdout, "stop:") is not None

    app("migrate")
    sql("""
        CREATE ROLE tenant_receipts LOGIN;
        CREATE SCHEMA tenant_app;
        CREATE TABLE tenant_app.receipts (receipt_key text PRIMARY KEY);
        GRANT USAGE ON SCHEMA tenant_app TO tenant_receipts;
        GRANT INSERT (receipt_key) ON tenant_app.receipts TO tenant_receipts;
        ALTER ROLE tenant_receipts SET statement_timeout = '8s';
        ALTER ROLE tenant_receipts SET lock_timeout = '6s';
    """)
    denied = run(["psql", "-XAt", app_url, "-c", "SELECT state FROM mesh_jobs.jobs"], check=False)
    assert denied.returncode != 0 and "permission denied" in denied.stderr
    for action, invalid, error in (
        ("serve", config(execution=TRUSTED, drain=True), "drain runners require fresh-process"),
        ("drain", config(execution=TRUSTED), "Jobs.drain requires fresh-process"),
    ):
        denied = app(action, runner_config=invalid, check=False)
        assert denied.returncode == 1 and error in denied.stderr, denied.stderr

    try:
        example = start("example")
        enqueue("example")
        reaches("example", "completed")
        assert effects("example") == 1
        stop_runner(example)

        for mode in ({}, {"execution": "fresh_process"}):
            strict = start(config(**mode))
            key = "strict-explicit" if mode else "strict-default"
            enqueue(key)
            reaches(key, "completed")
            assert effects(key) == 1
            assert f"handler:{key}" in strict[3].read_text()
            assert f"handler:{key}" not in strict[2].read_text()
            stop_runner(strict)

        trusted = start(config(execution=TRUSTED))
        for key, timeout in (("gc", 8000), ("panic", 8000), ("timeout", 400)):
            enqueue(key, timeout)
            completed = reaches(key, "completed")
            assert completed[2] == (1 if key == "gc" else 2), completed
            assert effects(key) == 1 and f"handler:{key}" in trusted[2].read_text()

        parked = enqueue("parked-cancel")
        wait("handler parked", lambda: line(trusted[2], "handler:parked-cancel") is not None)
        app("control", dict(action="cancel", id=parked[0]))
        reaches("parked-cancel", "cancelled")
        assert effects("parked-cancel") == 0

        enqueue("lost-owner")
        wait("effect committed before exit", lambda: effects("lost-owner") == 1)
        assert job("lost-owner")[1] == "executing"
        original_effect = sql("SELECT xmin FROM tenant_app.receipts WHERE receipt_key='lost-owner'")
        trusted[0].send_signal(signal.SIGKILL)
        trusted[0].wait(timeout=10)
        replacement = start(config(execution=TRUSTED))
        assert reaches("lost-owner", "completed")[2] == 2
        assert effects("lost-owner") == 1
        assert original_effect == sql("SELECT xmin FROM tenant_app.receipts WHERE receipt_key='lost-owner'")

        with (directory / "lock.out").open("w+") as lock_log:
            holder = subprocess.Popen(psql, stdin=subprocess.PIPE, stdout=lock_log, stderr=lock_log,
                                      text=True)
            children.append(holder)
            holder.stdin.write("BEGIN; LOCK tenant_app.receipts IN ACCESS EXCLUSIVE MODE;\n"
                               "\\echo held\n")
            holder.stdin.flush()
            wait("application table locked", lambda: "held\n" in Path(lock_log.name).read_text())
            blocked = enqueue("late-sql")
            wait("application SQL blocked", lambda: sql("""
                SELECT 1 FROM pg_stat_activity
                WHERE usename='tenant_receipts' AND wait_event_type='Lock'
                    AND query LIKE 'INSERT INTO tenant_app.receipts%'
            """) == "1")
            app("control", dict(action="cancel", id=blocked[0]))
            replacement[1].touch()
            report = wait("stop report with a blocked actor", lambda: line(replacement[2], "stop:"))
            assert json.loads(report)["undrained_jobs"] == [blocked[0]]
            assert job("late-sql")[1] == "executing"
            assert replacement[0].poll() is None
            holder.communicate("ROLLBACK;\n\\q\n", timeout=10)
            assert holder.returncode == 0
        stop_runner(replacement)
        reaches("late-sql", "cancelled")
        assert effects("late-sql") == 1
    except (AssertionError, RuntimeError, subprocess.TimeoutExpired):
        for path in sorted(directory.glob("runner-*.*")):
            if path.suffix in {".out", ".err"}:
                print(f"{path.name}\n{path.read_text()[-4000:]}", flush=True)
        raise
    finally:
        for child in children:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("meshc", type=Path)
    parser.add_argument("--runtime", type=Path, help="path to libmesh_rt.a from the verified build")
    args = parser.parse_args()
    for variable in ("PGHOSTADDR", "PGSERVICE", "PGOPTIONS"):
        os.environ.pop(variable, None)
    if "PG_BIN_DIR" in os.environ:
        os.environ["PATH"] = os.environ["PG_BIN_DIR"] + os.pathsep + os.environ["PATH"]
    scratch = ROOT / ".tmp"
    scratch.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="trusted-jobs-", dir=scratch) as directory:
        directory = Path(directory)
        project = directory / "project"
        project.mkdir()
        doc = (ROOT / "website/docs/docs/concurrency/index.md").read_text()
        section = doc.split("#### Run trusted handlers in the application process", 1)[1]
        example = section.split("```mesh\n", 1)[1].split("```", 1)[0]
        (project / "main.mpl").write_text(example + "\n" + (HERE / "fixture.mpl").read_text())
        binary = directory / "runner"
        env = dict(os.environ, TMPDIR=str(directory))
        if args.runtime:
            env["MESH_RT_LIB_PATH"] = str(args.runtime.resolve())
        run([str(args.meshc.resolve()), "build", str(project), "--opt-level", "2",
             "--output", str(binary)], env=env)
        data = directory / "pg"
        run(["initdb", "-D", str(data), "-U", "proof_owner", "--auth=trust",
             "--no-locale", "--encoding=UTF8"], env=env)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        run(["pg_ctl", "-D", str(data), "-l", str(directory / "postgres.log"), "-w",
             "-o", f"-p {port} -h 127.0.0.1 -k ''", "start"], env=env)
        try:
            uri = f"postgres://proof_owner@127.0.0.1:{port}/postgres?sslmode=disable"
            exercise(directory, binary, uri)
        finally:
            run(["pg_ctl", "-D", str(data), "-w", "-m", "immediate", "stop"], env=env)
    print("Trusted Jobs guide, strict execution, GC, panic, timeout, cancel, late SQL, and owner loss passed.")


if __name__ == "__main__":
    main()
