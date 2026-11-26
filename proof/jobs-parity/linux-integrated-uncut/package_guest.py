"""Prove named effects on the exact ELF staged as /proof/mesh-current."""

import json
import os
from pathlib import Path
import secrets
import selectors
import signal
import socket
import threading
import time

from jobs_parity_fault import _LoopbackProxy
from trial import Cluster, PROOF, Subject, h


class EffectReceiptGate(_LoopbackProxy):
    """Withhold replies on one attested loopback session while passing TLS unchanged."""

    def __init__(self, upstream_port):
        super().__init__(upstream_port)
        self.effect_port = None
        self.armed = threading.Event()
        self.close_effect = threading.Event()
        self.closed_effect = threading.Event()
        self.withheld_bytes = 0

    def _relay(self, client):
        try:
            with client, socket.create_connection(("127.0.0.1", self._upstream_port), timeout=5) as server:
                client.settimeout(5)
                port = server.getsockname()[1]
                with selectors.DefaultSelector() as selector:
                    selector.register(client, selectors.EVENT_READ, server)
                    selector.register(server, selectors.EVENT_READ, client)
                    while not self._stop.is_set():
                        gated = port == self.effect_port
                        if gated:
                            self.armed.set()
                            if self.close_effect.is_set():
                                self.closed_effect.set()
                                return
                        for key, _ in selector.select(0.015):
                            data = key.fileobj.recv(65536)
                            if not data:
                                return
                            if key.fileobj is server and port == self.effect_port:
                                self.withheld_bytes += len(data)
                            else:
                                key.data.sendall(data)
        except OSError:
            return


def command_line(pid):
    return Path(f"/proc/{pid}/cmdline").read_bytes().split(b"\0")[:2]


def await_value(check, description, seconds=15):
    return h.wait_for(check, time.monotonic() + seconds, description, 0.015)


class Scenario:
    def __init__(self, cluster, path, name):
        self.cluster = cluster
        self.path = path
        self.name = name
        self.namespace = "parity_" + secrets.token_hex(8)
        path.mkdir()
        cluster.evidence = path
        cluster.prepare_database("mesh")
        self.status = path / "runner"
        env = dict(cluster.env, TMPDIR=str(path), MESH_SCHEDULER_MIN_WORKERS="2",
                   MESH_SCHEDULER_MAX_WORKERS="2", MESH_EFFECT_LANES="1",
                   MESH_EFFECT_APP_URL=cluster.url("mesh", "compiled_caller"),
                   JOBS_PARITY_MESH_BATCH="1", JOBS_PARITY_MESH_LISTEN="1",
                   JOBS_PARITY_STATUS_PATH=str(self.status), MESH_DIAGNOSTIC_JOB_DISABLE_THP="1")
        self.subject = Subject("mesh", [str(PROOF / "mesh-current")], PROOF, cluster, env)
        self.env = self.subject.environment(self.namespace, 1, 1, "fault", 250)
        self.env["JOBS_PARITY_STATUS_PATH"] = str(self.status)
        self.subject.call("migrate", self.env, path / "migrate.log")
        assert cluster.owned("mesh")
        self.marks = path / "marks"
        for attempt in (1, 2):
            (self.marks / str(attempt)).mkdir(parents=True)
        self.runners = []

    def sql(self, query):
        return self.cluster.sql("mesh", query)

    def launch(self, env=None):
        for suffix in ("", ".request", ".pending"):
            Path(str(self.status) + suffix).unlink(missing_ok=True)
        index = len(self.runners)
        log = self.path / f"runner-{index}.log"
        runner = self.subject.run(env or self.env, index, log)
        self.runners.append(runner)
        await_value(lambda: h.runner_ready([runner], [log]), "runner admission")
        assert command_line(runner.pid) == [b"/proof/mesh-current", b"run"]
        return runner

    def cleanup(self):
        h.stop_runners([runner for runner in self.runners if runner.poll() is None], require_clean=False)

    def enqueue(self, timeout=None, attempts=None):
        batch = {"request_id": self.name, "rollback": False, "jobs": [
            {"key": self.name, "payload": "frozen package proof", "hold_ms": 0,
             "fail_first": False, "proof_path": str(self.marks)}]}
        path, = h.batch_files(self.path, [batch])
        # Set the exact deadline/attempt budget before any runner can claim.
        receipt, committed = self.subject.enqueue_receipt(self.env, path, self.path / "enqueue.log")
        assert committed is not None and set(receipt["ids"]) == {self.name}
        self.id = receipt["ids"][self.name]
        if timeout is not None:
            self.sql(f"UPDATE mesh_jobs.jobs SET timeout_ms={timeout}, max_attempts={attempts} "
                     f"WHERE namespace='{self.namespace}' AND id='{self.id}'")
            assert self.state("available", 0)
        return committed

    def marker(self, attempt, name):
        return self.marks / str(attempt) / name

    def read_mark(self, attempt, name):
        path = self.marker(attempt, name)
        return await_value(lambda: path.read_text() if path.is_file() else "", name)

    def worker(self, attempt):
        pid = int(self.read_mark(attempt, "worker"))
        argv = command_line(pid)
        assert argv == [b"/proc/self/exe", b"--mesh-internal-job-attempt-effect-v1"], argv
        assert h.process_table()[pid][0] == self.runners[-1].pid
        return pid

    def open_effect(self, attempt):
        pid = self.worker(attempt)
        self.marker(attempt, "call").touch(exist_ok=False)
        return pid, json.loads(self.read_mark(attempt, "first"))["outcome"]

    def complete(self, attempt):
        self.marker(attempt, "second-call").touch(exist_ok=False)
        assert self.read_mark(attempt, "second") == "applied"
        self.marker(attempt, "finish").touch(exist_ok=False)

    def state(self, state, attempt):
        row = self.cluster.claim_identity("mesh", self.namespace, self.name)
        if row and row["state"] == state and row["attempt"] == attempt:
            assert row["id"] == self.id
            return row

    def ledger(self):
        return json.loads(self.sql(f"""
        SELECT json_build_object(
          'first', (SELECT json_build_object('pid', worker_pid, 'attempt', attempt,
             'xid', xmin::text, 'commit', pg_xact_commit_timestamp(xmin))
             FROM public.mesh_jobs_parity_attempts WHERE namespace='{self.namespace}'),
          'second', (SELECT json_build_object('attempt', attempt, 'xid', xmin::text,
             'commit', pg_xact_commit_timestamp(xmin))
             FROM public.mesh_jobs_parity_effects WHERE namespace='{self.namespace}'))
        """))

    def assert_recovered(self, pid, terminal, first):
        await_value(lambda: self.state(terminal, 2), "replacement settlement")
        assert pid not in h.process_table()
        result = self.ledger()
        assert result["first"] == first and result["second"]["attempt"] == 2
        assert first["xid"] != result["second"]["xid"] and first["commit"] < result["second"]["commit"]
        return result


def deadline(probe):
    probe.enqueue(timeout=3000, attempts=1)
    probe.launch()
    pid, outcome = probe.open_effect(1)
    first = probe.ledger()
    assert outcome == "applied" and first["first"]["pid"] == pid and first["second"] is None
    terminal = await_value(lambda: probe.state("discarded", 1), "handler deadline settlement")
    reason = probe.sql(f"SELECT errors->0->>'reason' FROM mesh_jobs.jobs WHERE id='{probe.id}'")
    assert reason == "job handler timed out"
    assert probe.ledger() == first and pid not in h.process_table()
    return {"pid": pid, "terminal": terminal, "reason": reason, "ledger": first}


def stop(probe):
    probe.enqueue(timeout=20000, attempts=1)
    runner = probe.launch()
    pid, outcome = probe.open_effect(1)
    assert outcome == "applied"
    first = probe.ledger()["first"]
    runner.send_signal(signal.SIGTERM)
    time.sleep(0.10)
    assert runner.poll() is None and pid in h.process_table()
    probe.complete(1)
    assert runner.wait(timeout=20) == 0
    terminal = probe.state("completed", 1)
    ledger = probe.ledger()
    assert terminal and ledger["first"] == first and pid not in h.process_table()
    assert ledger["second"]["attempt"] == 1 and first["commit"] < ledger["second"]["commit"]
    return {"pid": pid, "runner_exit": runner.returncode, "terminal": terminal, "ledger": ledger}


def renew_and_recover(probe):
    probe.enqueue(timeout=20000, attempts=2)
    runner = probe.launch()
    first_pid, outcome = probe.open_effect(1)
    assert outcome == "applied"
    initial = probe.state("executing", 1)
    first = probe.ledger()["first"]
    query = f"SELECT lease_expires_at_ms FROM mesh_jobs.jobs WHERE id='{probe.id}'"
    expires = int(probe.sql(query))

    def renewed():
        current = int(probe.sql(query))
        return current if current > expires else None

    later = await_value(renewed, "lease renewal")
    assert probe.state("executing", 1) == initial
    runner.kill()
    assert runner.wait(timeout=5) == -signal.SIGKILL
    await_value(lambda: first_pid not in h.process_table(), "worker exits after owner loss")
    assert probe.ledger() == {"first": first, "second": None}
    probe.launch()
    pid, outcome = probe.open_effect(2)
    replacement = probe.state("executing", 2)
    assert outcome == "prior" and pid != first_pid and replacement["fence"] != initial["fence"]
    stale = probe.sql(f"SELECT mesh_jobs.settle('{probe.namespace}','{probe.id}',"
                      f"'{initial['fence']}', '{{\"action\":\"complete\"}}'::jsonb, false)")
    assert stale == "f" and probe.state("executing", 2) == replacement
    probe.complete(2)
    result = probe.assert_recovered(pid, "completed", first)
    return {"initial": initial, "expires_before_ms": expires, "expires_after_ms": later,
            "replacement": replacement, "stale_fence_rejected": True,
            "old_worker": first_pid, "replacement_worker": pid, "ledger": result}


def lost_acknowledgment(probe):
    probe.enqueue(timeout=20000, attempts=2)
    with EffectReceiptGate(probe.cluster.port) as gate:
        app_url = probe.cluster.url("mesh", "compiled_caller", port=gate.port)
        jobs_url = probe.cluster.url("mesh", port=gate.port)
        runner = probe.launch(dict(probe.env, MESH_EFFECT_APP_URL=app_url, JOBS_PARITY_POOL_URL=jobs_url))
        pid = probe.worker(1)
        # The worker is waiting for its call permit and the app session is idle.
        # The backend's client port identifies the exact upstream TLS tunnel.
        session = json.loads(probe.sql("""
            SELECT json_agg(json_build_object('port', a.client_port, 'tls', s.ssl, 'state', a.state))
            FROM pg_stat_activity AS a JOIN pg_stat_ssl AS s USING (pid)
            WHERE a.datname = current_database() AND a.usename = 'compiled_caller'
        """))
        app, = session
        assert app["tls"] and app["state"] == "idle" and app["port"] > 0
        gate.effect_port = app["port"]
        assert gate.armed.wait(timeout=5)
        probe.marker(1, "call").touch(exist_ok=False)
        await_value(lambda: probe.ledger()["first"], "first effect committed while response is held")
        withheld = await_value(lambda: gate.withheld_bytes, "withheld server TLS response")
        gate.close_effect.set()
        assert gate.closed_effect.wait(timeout=5)
        outcome = json.loads(probe.read_mark(1, "first"))["outcome"]
        assert outcome == "Jobs effect outcome is unconfirmed"
        fault = {"placement": "after_durable_commit", "effect_session_tls": app["tls"],
                 "withheld_response_bytes": withheld}
        committed = probe.ledger()
        assert committed["first"]["pid"] == pid and committed["first"]["commit"]
        assert committed["second"] is None
        probe.marker(1, "finish").touch(exist_ok=False)
        assert probe.read_mark(1, "returned-complete") == outcome
        await_value(lambda: probe.state("retryable", 1), "unconfirmed effect leaves retryable job")
        assert pid not in h.process_table()
        h.stop_runners([runner], require_clean=True)
    probe.launch()
    replacement_pid, outcome = probe.open_effect(2)
    assert outcome == "prior" and replacement_pid != pid and probe.state("executing", 2)
    probe.complete(2)
    result = probe.assert_recovered(replacement_pid, "completed", committed["first"])
    return {"cut": fault, "first_worker": pid, "replacement_worker": replacement_pid,
            "first_attempt_returned_complete": True, "ledger": result}


def listen(probe):
    runner = probe.launch(dict(probe.env, JOBS_PARITY_POLL_MS="10000"))
    observer = h.ListenerTrial(probe.cluster, probe.namespace, probe.status, None, False)
    until = time.monotonic() + 15

    def subscribed():
        wake = observer.status(until)
        if wake["subscribed"] and wake["subscription_scans"] > 0:
            return wake

    before = await_value(subscribed, "subscription scan")
    sessions = observer.sessions()
    assert sum(row["listener"] for row in sessions) == 1 and all(row["tls"] for row in sessions)
    probe.enqueue()
    pid, outcome = probe.open_effect(1)
    assert outcome == "applied"
    probe.complete(1)
    await_value(lambda: probe.state("completed", 1), "notified job settles")
    after = observer.status(time.monotonic() + 15)
    assert after["notification_scans"] > before["notification_scans"]
    assert after["subscriptions"] == before["subscriptions"] and observer.sessions() == sessions
    assert pid not in h.process_table()
    h.stop_runners([runner], require_clean=True)
    return {"before": before, "after": after, "tls_sessions": sessions, "ledger": probe.ledger()}


def run(config, root, evidence):
    assert os.uname().machine == "aarch64" and os.getuid() == 65534
    assert os.confstr("CS_GNU_LIBC_VERSION").startswith("glibc ")
    path = PROOF / "mesh-current"
    assert h.digest(path) == config["elf_sha256"]
    cluster_dir = root / "cluster"
    cluster_dir.mkdir()
    cluster = Cluster(cluster_dir, evidence, 68, "17.11")
    cluster.runners, cluster.slots, cluster.lanes = 1, 64, 1
    try:
        cluster.start()
        for check in (stop, deadline, renew_and_recover, lost_acknowledgment, listen):
            print(f"FROZEN_DIAGNOSTIC_ARM slow proof {check.__name__}", flush=True)
            probe = Scenario(cluster, root / check.__name__, check.__name__)
            try:
                result = check(probe)
                h.write_json(evidence / f"{check.__name__}.json", result)
            finally:
                for log in probe.path.glob("*.log"):
                    (evidence / f"{check.__name__}-{log.name}").write_bytes(log.read_bytes())
                probe.cleanup()
        h.write_json(evidence / "pass.json", {"elf_sha256": h.digest(path), "kernel": os.uname().release,
                                             "glibc": os.confstr("CS_GNU_LIBC_VERSION")})
    finally:
        cluster.evidence = evidence
        cluster.stop()
