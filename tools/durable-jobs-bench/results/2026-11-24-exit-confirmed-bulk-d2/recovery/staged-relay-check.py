"""Exercise the staged Relay as the benchmark UID with Python cache disabled."""
import json
import os
from pathlib import Path
import socket
import struct
import sys
from threading import Thread

import wire


def tcp_pair():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        front = socket.create_connection(listener.getsockname(), timeout=3)
        back, _ = listener.accept()
        back.settimeout(3)
        return front, back


def send(sock, kind, body):
    sock.sendall(kind + struct.pack("!I", len(body) + 4) + body)


def check():
    assert (os.getuid(), os.geteuid(), os.getgid()) == (65534, 65534, 65534)
    assert sys.dont_write_bytecode and sys.pycache_prefix
    assert not Path(sys.pycache_prefix).exists()
    source = Path(__file__).resolve().with_name("wire.py")
    assert Path(wire.__file__).resolve() == source
    assert wire.Relay.relay.__code__.co_filename == str(source)
    class Cluster:
        port = 5432
        certificate = Path(sys.argv[1]) / "cert.pem"
        data = Path(sys.argv[1])

    relay = wire.Relay(Cluster())
    client, frontend = tcp_pair()
    upstream, server = tcp_pair()
    errors = []
    def exchange():
        with frontend, upstream:
            try:
                relay.relay(frontend, upstream)
            except BaseException as error:
                errors.append(error)

    worker = Thread(target=exchange)
    worker.start()
    with client, server:
        startup = struct.pack("!II", 8, 196608)
        client.sendall(startup)
        assert wire.read(server, len(startup)) == startup
        send(server, b"Z", b"I")
        assert wire.frame(client)[0] == b"Z"
        port = server.getpeername()[1]
        for query in (b"SELECT 1", b"INSERT INTO proof VALUES (1)"):
            gate = relay.arm(query, drop=True) if query.startswith(b"INSERT") else None
            send(client, b"Q", query + b"\0")
            kind, _, message = wire.frame(server)
            assert kind == b"Q" and message == query + b"\0"
            send(server, b"C", b"INSERT 0 1\0" if gate else b"SELECT 1\0")
            assert wire.frame(client)[0] == b"C"
            send(server, b"Z", b"I")
            if gate:
                assert gate.hit.wait(2), "the staged relay did not hold the INSERT receipt"
                assert gate.witness == dict(command="INSERT 0 1", rows=0, idle=True, receipt_dropped=True)
                gate.release.set()
            else:
                assert wire.frame(client)[0] == b"Z"
                assert relay.requests == [(port, "SELECT 1")]
        assert client.recv(1) == b""
    worker.join(3)
    assert not worker.is_alive() and not errors, errors
    assert relay.requests == [(port, "SELECT 1"), (port, "INSERT INTO proof VALUES (1)")]
    assert not Path(sys.pycache_prefix).exists()
    status = Path("/proc/self/status").read_text()
    cap = int(next(line.split()[1] for line in status.splitlines() if line.startswith("CapEff:")), 16)
    assert cap == 0
    return dict(uid=os.getuid(), gid=os.getgid(), capabilities=cap, cpu_mask=sorted(os.sched_getaffinity(0)),
                bytecode_disabled=True, cache_absent=True, queries=[query for _, query in relay.requests],
                receipt_dropped=True)


if __name__ == "__main__":
    print(json.dumps(check(), sort_keys=True))
