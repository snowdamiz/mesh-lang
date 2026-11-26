#!/usr/bin/env python3
"""Check the opt-in set-based enqueue writer at the real PostgreSQL reply and COMMIT boundaries."""

import json
import os
from pathlib import Path
import socket
import ssl
import struct
import subprocess
import sys
from threading import Thread
from urllib.parse import urlsplit

TEST = "jobs::store::store_tests::set_based_enqueue_never_reports_an_uncertain_reply"
BATCH = b"FROM mesh_jobs.enqueue_batch_v1("
JOBS = 20
# PostgreSQL has committed the batch when the client sees each of these faults.
# Jobs.enqueue commits through the shared Jobs transaction helper. It detects a
# rolled-back COMMIT from the aborted transaction state, as PostgreSQL reports
# it, not from a rewritten tag, so a forged ROLLBACK tag is not a case here.
POST_COMMIT = {"commit_reply_lost", "after_command_complete"}
FAULTS = ("partial_row", "missing_row", "bad_ordinal", "bad_receipt", "before_commit",
          *sorted(POST_COMMIT))


def read(sock, size):
    body = bytearray()
    while len(body) < size:
        data = sock.recv(size - len(body))
        if not data:
            raise EOFError
        body.extend(data)
    return bytes(body)


def frame(sock):
    kind, header = read(sock, 1), read(sock, 4)
    size, = struct.unpack("!I", header)
    assert 4 <= size <= 32 * 1024**2
    return kind, header, read(sock, size - 4)


def send(sock, kind, body):
    sock.sendall(kind + struct.pack("!I", len(body) + 4) + body)



def fields(row):
    count, = struct.unpack_from("!H", row)
    values, offset = [], 2
    for _ in range(count):
        size, = struct.unpack_from("!i", row, offset)
        offset += 4
        values.append(None if size < 0 else row[offset:offset + size])
        offset += max(0, size)
    assert offset == len(row)
    return values


def row(values):
    return struct.pack("!H", len(values)) + b"".join(
        struct.pack("!i", -1) if value is None else struct.pack("!i", len(value)) + value
        for value in values
    )


class Relay:
    def __init__(self, upstream, pgdata, function, fault):
        self.upstream, self.function, self.fault = upstream, function, fault
        self.cut, self.error = False, None
        self.server = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self.server.load_cert_chain(pgdata / "server.crt", pgdata / "server.key")
        self.client = ssl.create_default_context(cafile=str(pgdata.parent / "ca.crt"))
        # The disposable CA uses minimal extensions. Keep chain and SAN validation.
        self.client.verify_flags &= ~ssl.VERIFY_X509_STRICT

    def __enter__(self):
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(1)
        self.listener.settimeout(12)
        self.port = self.listener.getsockname()[1]
        self.thread = Thread(target=self.relay)
        self.thread.start()
        return self

    def __exit__(self, exc, *_):
        self.thread.join(14)
        self.listener.close()
        assert not self.thread.is_alive(), "enqueue proof relay did not stop"
        if exc is None:
            assert self.error is None, self.error
            assert self.cut, "enqueue proof did not reach the requested reply"

    def relay(self):
        try:
            cli, _ = self.listener.accept()
            with cli, socket.create_connection(self.upstream, timeout=8) as pg:
                cli.settimeout(8)
                greeting = read(cli, 8)
                assert greeting == bytes.fromhex("0000000804d2162f")
                cli.sendall(b"S")
                pg.sendall(greeting)
                assert read(pg, 1) == b"S"
                with self.server.wrap_socket(cli, server_side=True) as cli, \
                        self.client.wrap_socket(pg, server_hostname="127.0.0.1") as pg:
                    self.authenticate(cli, pg)
                    self.transactions(cli, pg)
        except (EOFError, ConnectionResetError, BrokenPipeError, ssl.SSLEOFError) as error:
            if not self.cut:
                self.error = error
        except BaseException as error:
            self.error = error

    @staticmethod
    def authenticate(cli, pg):
        header = read(cli, 4)
        pg.sendall(header + read(cli, struct.unpack("!I", header)[0] - 4))
        while True:
            kind, header, body = frame(pg)
            cli.sendall(kind + header + body)
            if kind == b"R" and int.from_bytes(body[:4], "big") in (10, 11):
                pg.sendall(b"".join(frame(cli)))
            if kind == b"Z":
                return

    def transactions(self, cli, pg):
        query = b""
        offered = False
        while True:
            kind, header, body = frame(cli)
            if kind == b"P":
                query = body.split(b"\0", 2)[1]
            if kind == b"Q":
                query = body[:-1]
            commit = offered and query == b"COMMIT" and kind == b"Q"
            if commit and self.fault == "before_commit":
                self.cut = True
                return
            if commit:
                assert self.fault in POST_COMMIT, "client committed a damaged batch reply"
            pg.sendall(kind + header + body)
            if kind == b"X":
                return
            if kind not in (b"Q", b"S"):
                continue
            batch = BATCH in query and kind == b"S"
            offered |= BATCH in query
            if self.reply(cli, pg, batch, commit):
                return

    def reply(self, cli, pg, batch, commit):
        rows = 0
        while True:
            kind, header, body = frame(pg)
            if batch and kind == b"D":
                rows += 1
                if rows == 2:
                    values = fields(body)
                    assert len(values) == 2 and values[0] == b"2", values
                    if self.fault == "partial_row":
                        cli.sendall(kind + header + body[:len(body) // 2])
                        self.cut = True
                        return True
                    if self.fault == "missing_row":
                        self.cut = True
                        continue
                    if self.fault == "bad_ordinal":
                        values[0] = b"9"
                        self.cut = True
                    if self.fault == "bad_receipt":
                        receipt = json.loads(values[1])
                        assert receipt["inserted"] is True
                        receipt["inserted"] = False
                        values[1] = json.dumps(receipt).encode()
                        self.cut = True
                    body = row(values)
            if commit and kind == b"C":
                assert body == b"COMMIT\0"
                if self.fault == "commit_reply_lost":
                    self.cut = True
                    return True
            if commit and kind == b"Z" and self.fault == "after_command_complete":
                assert body == b"I"
                self.cut = True
                return True
            send(cli, kind, body)
            if kind == b"Z":
                return False


def sql(url, statement):
    result = subprocess.run(
        ["psql", "-XqAtw", "-v", "ON_ERROR_STOP=1", url.geturl()],
        input=statement, capture_output=True, text=True, timeout=12,
    )
    assert result.returncode == 0, result.stderr
    return result.stdout.strip()


def test_binary():
    build = subprocess.run(
        ["cargo", "test", "--locked", "-p", "mesh-rt", "--lib", "--no-run", "--message-format=json"],
        stdout=subprocess.PIPE, text=True, check=True,
    )
    return next(record["executable"] for record in map(json.loads, build.stdout.splitlines())
                if record.get("target", {}).get("name") == "mesh_rt" and record.get("executable"))


def witness(binary, url, ns, fault, replay=False):
    env = dict(os.environ, MESH_TEST_ENQUEUE_RELAY_URL=url,
               MESH_TEST_ENQUEUE_RELAY_NAMESPACE=ns, MESH_TEST_ENQUEUE_RELAY_CASE=fault)
    env.pop("MESH_TEST_ENQUEUE_RELAY_REPLAY", None)
    if replay:
        env["MESH_TEST_ENQUEUE_RELAY_REPLAY"] = "1"
    run = subprocess.run([binary, "--exact", TEST, "--ignored", "--nocapture"], env=env,
                         capture_output=True, text=True, timeout=13)
    assert run.returncode == 0 and "1 passed" in run.stdout, run.stdout + run.stderr
    return run.stdout


def stored(url, ns):
    return sql(url, f"""
        SELECT (SELECT count(*) FROM mesh_jobs.jobs WHERE namespace='{ns}') || '/' ||
               (SELECT count(*) FROM mesh_jobs.receipts WHERE namespace='{ns}')
    """)


def prove(url, pgdata, binary, fault):
    ns = f"enqueue_wire_{fault}"
    with Relay((url.hostname, url.port), pgdata, BATCH, fault) as relay:
        endpoint = url._replace(netloc=url.netloc.rsplit(":", 1)[0] + ":" + str(relay.port))
        witness(binary, endpoint.geturl(), ns, fault)
    expected = f"{JOBS}/{JOBS}" if fault in POST_COMMIT else "0/0"
    assert stored(url, ns) == expected, (fault, stored(url, ns))
    if fault in POST_COMMIT:
        # Resolving the uncertain COMMIT with the same request keys returns
        # the committed receipts and inserts nothing.
        receipts = json.loads(witness(binary, url.geturl(), ns, fault, replay=True)
                              .split("RECEIPTS ", 1)[1].splitlines()[0])
        ids = sql(url, f"SELECT string_agg(id::text, ',' ORDER BY position) FROM mesh_jobs.jobs WHERE namespace='{ns}'")
        assert [r["id"] for r in receipts] == ids.split(","), fault
        assert stored(url, ns) == expected, fault
    print(f"Jobs set-based enqueue wire {fault} passed", flush=True)


if __name__ == "__main__":
    pgdata = Path(sys.argv[1])
    url = urlsplit(os.environ["MESH_TEST_DATABASE_URL"])
    if sql(url, "SELECT current_setting('server_version_num')::int / 10000") != "17":
        print("Jobs set-based enqueue wire proof skipped: the writer installs on PostgreSQL 17 only")
        sys.exit(0)
    binary = test_binary()
    for fault in FAULTS:
        prove(url, pgdata, binary, fault)
