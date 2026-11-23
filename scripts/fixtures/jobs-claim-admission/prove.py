#!/usr/bin/env python3
"""Check the product batch claim reader at the real PostgreSQL COMMIT boundary."""

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

TEST = "jobs::store::store_tests::checked_grant_never_escapes_an_invalid_or_uncertain_reply"
POST_COMMIT = {"after_commit", "wrong_commit_tag"}


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


def maximum_context(row):
    # Each DataRow remains valid on its own. Together the two replies
    # exceed the batch's 32 MiB framed limit.
    count, = struct.unpack_from("!H", row)
    values, offset = [], 2
    for _ in range(count):
        size, = struct.unpack_from("!i", row, offset)
        offset += 4
        values.append(None if size < 0 else row[offset:offset + size])
        offset += max(0, size)
    assert count == 8 and offset == len(row)
    envelope = json.loads(values[3])
    envelope["args"] = {"payload": ""}
    text = json.dumps(envelope).encode()
    context = text.replace(b'"payload": ""',
                           b'"payload": "' + b"x" * (16 * 1024**2 - len(text)) + b'"', 1)
    assert len(context) == 16 * 1024**2
    values[3] = context
    values[5] = str(len(context)).encode()
    return struct.pack("!H", count) + b"".join(
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
        assert not self.thread.is_alive(), "claim proof relay did not stop"
        if exc is None:
            assert self.error is None, self.error
            assert self.cut, "claim proof did not reach the requested reply"

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
                assert self.fault in POST_COMMIT, "client accepted a damaged grant"
            pg.sendall(kind + header + body)
            if kind == b"X":
                return
            if kind not in (b"Q", b"S"):
                continue
            claim = self.function in query
            offered |= claim
            if self.reply(cli, pg, claim, commit):
                return

    def reply(self, cli, pg, claim, commit):
        rows = 0
        while True:
            kind, header, body = frame(pg)
            if claim and kind == b"D":
                rows += 1
                if self.fault == "reply_budget":
                    body = maximum_context(body)
                    self.cut = rows == 2
                if rows == 2:
                    if self.fault == "partial_row":
                        cli.sendall(kind + header + body[:len(body) // 2])
                        self.cut = True
                        return True
                    if self.fault == "missing_row":
                        self.cut = True
                        continue
                    if self.fault in {"bad_ordinal", "bad_context"}:
                        old, new = ((b"2", b"9") if self.fault == "bad_ordinal"
                                    else (b'"queue": "default"', b'"queue": "invalid"'))
                        assert old in body
                        if self.fault == "bad_ordinal":
                            assert body.endswith(b"\0\0\0\x012")
                            body = body[:-1] + new
                        else:
                            body = body.replace(old, new, 1)
                        self.cut = True
            if claim and kind == b"C" and self.fault == "wrong_select_count":
                assert body == b"SELECT 2\0"
                body = b"SELECT 1\0"
                self.cut = True
            if claim and kind == b"Z" and self.fault == "bad_ready":
                assert body == b"T"
                send(cli, kind, b"")
                self.cut = True
                return True
            if commit and kind == b"C":
                assert body == b"COMMIT\0"
                if self.fault == "wrong_commit_tag":
                    body = b"ROLLBACK\0"
                    self.cut = True
            if commit and kind == b"Z" and self.fault == "after_commit":
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


def prove(url, pgdata, binary, route, fault):
    ns = f"wire_{route}_{fault}"
    sql(url, f"""
        INSERT INTO mesh_jobs.queues(namespace,name,concurrency) VALUES ('{ns}','default',2);
        INSERT INTO mesh_jobs.jobs (namespace,worker,version,queue,args,metadata,tags,state,
            priority,max_attempts,timeout_ms,backoff_base_ms,backoff_max_ms,inserted_at_ms,scheduled_at_ms)
        SELECT '{ns}','Deliver',1,'default','{{}}','{{}}','[]','available', n, n,60000,10000,10000,0,n
        FROM generate_series(1,2) AS n;
    """)
    function = b"FROM mesh_jobs.claim_batch" + (b"_set_v1(" if route == "set" else b"_v1(")
    with Relay((url.hostname, url.port), pgdata, function, fault) as relay:
        endpoint = url._replace(netloc=url.netloc.rsplit(":", 1)[0] + ":" + str(relay.port))
        env = dict(os.environ, MESH_TEST_CLAIM_RELAY_URL=endpoint.geturl(),
                   MESH_TEST_CLAIM_RELAY_NAMESPACE=ns, MESH_TEST_CLAIM_RELAY_CASE=fault)
        env.pop("MESH_TEST_CLAIM_RELAY_CURSOR", None)
        if route == "cursor":
            env["MESH_TEST_CLAIM_RELAY_CURSOR"] = "1"
        witness = subprocess.run([binary, "--exact", TEST, "--ignored"], env=env,
                                 capture_output=True, text=True, timeout=13)
        assert witness.returncode == 0 and "1 passed" in witness.stdout, witness.stdout + witness.stderr
    history = sql(url, f"""
        SELECT string_agg(state || ':' || attempt || ':' || (lease_token IS NOT NULL),
                          ',' ORDER BY priority)
        FROM mesh_jobs.jobs WHERE namespace='{ns}'
    """)
    if fault in POST_COMMIT:
        assert history == "executing:1:true,executing:1:true", history
        recovered = sql(url, f"""
            UPDATE mesh_jobs.jobs SET lease_expires_at_ms=mesh_jobs.now_ms()-1 WHERE namespace='{ns}';
            SELECT mesh_jobs.recover('{ns}',2,NULL);
            SELECT string_agg(state,',' ORDER BY priority) FROM mesh_jobs.jobs WHERE namespace='{ns}'
        """)
        assert recovered == "2\ndiscarded,retryable", recovered
    else:
        assert history == "available:0:false,available:0:false", history
    print(f"Jobs claim wire {route} {fault} passed", flush=True)


if __name__ == "__main__":
    pgdata = Path(sys.argv[1])
    url = urlsplit(os.environ["MESH_TEST_DATABASE_URL"])
    binary = test_binary()
    for route in ("set", "cursor"):
        for fault in ("partial_row", "missing_row", "bad_ordinal", "bad_context",
                      "wrong_select_count", "bad_ready", "reply_budget", "before_commit", "after_commit",
                      "wrong_commit_tag"):
            prove(url, pgdata, binary, route, fault)
