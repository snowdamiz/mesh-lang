"""Recheck prepared and activation truncation and commit uncertainty on the public runner."""

from contextlib import contextmanager
import importlib.util
import json
import os
from pathlib import Path
import socket
from socketserver import BaseRequestHandler, ThreadingTCPServer
import ssl
import struct
import sys
from tempfile import TemporaryDirectory
from threading import Event, Thread
from time import monotonic
from urllib.parse import urlsplit, urlunsplit

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE / "adapter"))
from backend import Cluster, Subject, arm

reference = (Path(os.environ["PRODUCT_SOURCE"]) / "scripts/fixtures/jobs-claim-admission/prove.py")
spec = importlib.util.spec_from_file_location("checked_claim_proof", reference)
wire = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wire)

# The runner capability probe names both functions inside to_regprocedure().
PREPARE_CALL = b"FROM mesh_jobs.prepare_batch_v1("
ACTIVATE_CALL = b"FROM mesh_jobs.activate_batch_v1("


class Connection(BaseRequestHandler):
    def handle(self):
        relay = self.server
        try:
            with socket.create_connection(relay.upstream, timeout=8) as server:
                self.request.settimeout(8)
                greeting = wire.read(self.request, 8)
                assert greeting == bytes.fromhex("0000000804d2162f")
                self.request.sendall(b"S")
                server.sendall(greeting)
                assert wire.read(server, 1) == b"S"
                with relay.server_tls.wrap_socket(self.request, server_side=True) as client, \
                        relay.client_tls.wrap_socket(server, server_hostname="127.0.0.1") as pg:
                    wire.Relay.authenticate(client, pg)
                    self.forward(client, pg)
        except (EOFError, ConnectionError, TimeoutError, ssl.SSLEOFError):
            return
        except BaseException as error:
            relay.error = type(error).__name__
            relay.done.set()

    def forward(self, client, pg):
        relay = self.server
        sql = b""
        activated = False
        while True:
            kind, header, body = wire.frame(client)
            if kind == b"X":
                return
            if kind == b"P":
                sql = body.split(b"\0", 2)[1]
                if PREPARE_CALL in sql and relay.done.is_set():
                    return
                if ACTIVATE_CALL in sql:
                    relay.activations += 1
            elif kind == b"Q":
                sql = body.rstrip(b"\0")
                if sql == b"ROLLBACK" and relay.done.is_set():
                    relay.rollback.set()
            pg.sendall(kind + header + body)
            if kind not in (b"Q", b"S"):
                continue
            preparation = PREPARE_CALL in sql
            activation = ACTIVATE_CALL in sql
            rows = 0
            while True:
                tag, header, body = wire.frame(pg)
                if tag == b"D":
                    rows += 1
                    if ((preparation and rows == 2 and relay.fault == "prepare") or
                            (activation and rows == 3 and relay.fault == "activate")):
                        relay.done.set()
                        continue
                if activation and tag == b"Z" and rows == 3:
                    activated = True
                if sql == b"COMMIT" and activated and tag == b"Z" and relay.fault == "commit":
                    assert body == b"I"
                    relay.done.set()
                    return
                client.sendall(tag + header + body)
                if tag == b"Z":
                    break


@contextmanager
def fault_proxy(cluster, fault):
    with ThreadingTCPServer(("127.0.0.1", 0), Connection) as relay:
        relay.daemon_threads = True
        relay.upstream = ("127.0.0.1", cluster.port)
        relay.fault, relay.done, relay.rollback = fault, Event(), Event()
        relay.activations, relay.error = 0, None
        relay.server_tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        relay.server_tls.load_cert_chain(cluster.data / "server.crt", cluster.data / "server.key")
        relay.client_tls = ssl.create_default_context(cafile=str(cluster.certificate))
        relay.client_tls.verify_flags &= ~ssl.VERIFY_X509_STRICT
        thread = Thread(target=relay.serve_forever)
        thread.start()
        try:
            yield relay
        finally:
            relay.shutdown()
            thread.join()


def trial(cluster, runner, out, fault, ordinal):
    h = arm.h
    ns = f"parity_{ordinal:016x}"
    subject = Subject("mesh", [str(runner)], runner.parent, cluster, dict(
        cluster.env, MESH_SCHEDULER_MIN_WORKERS="2", MESH_SCHEDULER_MAX_WORKERS="2",
        JOBS_PARITY_MESH_BATCH="1", JOBS_PARITY_MESH_LISTEN="1", MESH_EFFECT_LANES="1",
        MESH_EFFECT_APP_URL=cluster.url("mesh", "compiled_caller")))
    env = subject.environment(ns, 2, 2, "dense", 250)
    subject.call("migrate", env, out / "migrate.log")
    cluster.sql("mesh", f"""
        INSERT INTO mesh_jobs.jobs(namespace, worker, version, queue, args, metadata, tags,
            state, priority, max_attempts, timeout_ms, backoff_base_ms, backoff_max_ms,
            inserted_at_ms, scheduled_at_ms)
        SELECT '{ns}', 'parity', 1, 'default',
            jsonb_build_object('key', 'fault_' || i, 'payload', repeat('p', 512*1024),
                               'hold_ms', 0, 'fail_first', false),
            '{{}}', '[]', 'available', i, 1, 10000, 0, 0, mesh_jobs.now_ms(), 0
        FROM generate_series(1, 2) i
    """)
    log = out / "runner.log"
    with fault_proxy(cluster, fault) as proxy:
        parsed = urlsplit(cluster.url("mesh"))
        netloc = parsed.netloc.rsplit(":", 1)[0] + ":" + str(proxy.server_address[1])
        env["JOBS_PARITY_POOL_URL"] = urlunsplit(parsed._replace(netloc=netloc))
        child = subject.run(env, 0, log)
        try:
            h.wait_for(lambda: h.runner_ready([child], [log]), monotonic() + 8, "runner readiness")
            assert proxy.done.wait(8) and proxy.error is None
            if fault != "commit":
                assert proxy.rollback.wait(5)
            assert proxy.activations == (0 if fault == "prepare" else 1)
        finally:
            h.stop_runners([child], require_clean=True)
    state = json.loads(cluster.sql("mesh", f"""
        SELECT json_agg(json_build_array(state, attempt, lease_token IS NOT NULL) ORDER BY position)
        FROM mesh_jobs.jobs WHERE namespace='{ns}'
    """))
    expected = ["executing", 1, True] if fault == "commit" else ["available", 0, False]
    assert state == [expected, expected], state
    assert cluster.sql("mesh", "SELECT count(*) FROM public.mesh_jobs_parity_attempts") == "0"
    assert cluster.sql("mesh", "SELECT count(*) FROM public.mesh_jobs_parity_effects") == "0"
    report = h.mesh_shutdown_report(log)
    assert report["last_batch_confirmed_us"] == report["claimed"] == report["finished"] == 0
    return dict(fault=fault, state=state, activations=proxy.activations, launched=0)


def prove(runner, out):
    reports = []
    for index, fault in enumerate(("prepare", "activate", "commit"), 1):
        case = out / fault
        case.mkdir()
        with TemporaryDirectory(dir=out, prefix="pg-") as directory:
            cluster = Cluster(Path(directory), case, 96)
            cluster.trusted_application = True
            cluster.runners, cluster.slots, cluster.lanes = 1, 2, 1
            try:
                cluster.start()
                cluster.prepare_database("mesh")
                reports.append(trial(cluster, runner, case, fault, index))
            finally:
                cluster.stop()
                for log in case.glob("*.log"):
                    log.unlink()
    return reports


if __name__ == "__main__":
    runner, out = (Path(value).resolve() for value in sys.argv[1:])
    out.mkdir()
    result = dict(runner_sha256=arm.h.digest(runner), cases=prove(runner, out))
    (out / "result.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
