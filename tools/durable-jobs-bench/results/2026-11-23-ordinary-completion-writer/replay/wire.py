"""Withhold an idle PostgreSQL receipt in a private loopback TLS fixture."""
from dataclasses import dataclass, field
import socket
import ssl
import struct
import sys
from threading import Event, Lock, Thread

SSL_REQUEST = struct.pack("!II", 8, 80877103)


def read(sock, count):
    data = bytearray()
    while len(data) < count:
        chunk = sock.recv(count - len(data))
        if not chunk:
            raise EOFError
        data.extend(chunk)
    return bytes(data)


def frame(sock):
    kind, header = read(sock, 1), read(sock, 4)
    size, = struct.unpack("!I", header)
    assert 4 <= size <= 17 * 1024**2
    return kind, header, read(sock, size - 4)


@dataclass
class Gate:
    query: bytes
    drop: bool
    hit: Event = field(default_factory=Event)
    release: Event = field(default_factory=Event)
    witness: dict | None = None


class Relay:
    def __init__(self, cluster):
        self.upstream = cluster.port
        self.client_context = ssl.create_default_context(cafile=str(cluster.certificate))
        # Match libpq verify-full for the fixture certificate. Python 3.13+
        # also requires RFC 5280 extensions that this throwaway CA omits.
        self.client_context.verify_flags &= ~ssl.VERIFY_X509_STRICT
        self.server_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self.server_context.load_cert_chain(cluster.data / "server.crt", cluster.data / "server.key")
        self.stop = Event()
        self.lock = Lock()
        self.gates = []
        self.errors = []

    def __enter__(self):
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen()
        self.listener.settimeout(0.2)
        self.port = self.listener.getsockname()[1]
        self.children = []
        self.thread = Thread(target=self.accept)
        self.thread.start()
        return self

    def __exit__(self, exc, *_):
        self.stop.set()
        self.thread.join(2)
        self.listener.close()
        for gate in self.gates:
            gate.release.set()
        for thread in self.children:
            thread.join(7)
        assert not any(thread.is_alive() for thread in self.children), "relay left a live session"
        if self.errors:
            print("private TLS relay:", self.errors, file=sys.stderr)
        if exc is None:
            assert not self.errors, self.errors
            assert all(gate.hit.is_set() for gate in self.gates)

    def arm(self, query, drop=False):
        gate = Gate(query, drop)
        with self.lock:
            self.gates.append(gate)
        return gate

    def accept(self):
        while not self.stop.is_set():
            try:
                cli, _ = self.listener.accept()
            except TimeoutError:
                continue
            thread = Thread(target=self.connection, args=(cli,))
            self.children.append(thread)
            thread.start()

    def connection(self, raw):
        try:
            with raw, socket.create_connection(("127.0.0.1", self.upstream), timeout=5) as up:
                raw.settimeout(5)
                for socket_ in (raw, up):
                    socket_.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                assert read(raw, 8) == SSL_REQUEST
                raw.sendall(b"S")
                up.sendall(SSL_REQUEST)
                assert read(up, 1) == b"S"
                with (self.server_context.wrap_socket(raw, server_side=True) as cli,
                      self.client_context.wrap_socket(up, server_hostname="127.0.0.1") as pg):
                    self.relay(cli, pg)
        except (EOFError, BrokenPipeError, ConnectionResetError, TimeoutError):
            pass
        except BaseException as error:
            self.errors.append(repr(error))

    def relay(self, cli, pg):
        header = read(cli, 4)
        size, = struct.unpack("!I", header)
        assert 8 <= size <= 10_000
        pg.sendall(header + read(cli, size - 4))
        while True:
            kind, header, data = frame(pg)
            cli.sendall(kind + header + data)
            if kind == b"R" and data[:4] in (b"\0\0\0\x0a", b"\0\0\0\x0b"):
                reply = frame(cli)
                assert reply[0] == b"p"
                pg.sendall(b"".join(reply))
            if kind == b"Z":
                break

        query = b""
        admitted = False
        while True:
            kind, header, data = frame(cli)
            if kind == b"P":
                query = data.split(b"\0", 2)[1]
            elif kind == b"Q":
                query = data[:-1]
            pg.sendall(kind + header + data)
            if kind == b"X":
                return
            if kind not in (b"Q", b"S"):
                continue
            count = 0
            grant_rows = 0
            command = None
            while True:
                kind, header, data = frame(pg)
                count += kind == b"D"
                if kind == b"D" and b"FROM mesh_jobs.claim_admitted_v3(" in query:
                    length = struct.unpack("!i", data[2:6])[0]
                    grant_rows += data[6:6 + length] == b"claimed"
                if kind == b"C":
                    command = data[:-1].decode()
                if kind == b"Z":
                    gate = None
                    if data == b"I" and command:
                        with self.lock:
                            gate = next((gate for gate in self.gates if not gate.hit.is_set()
                                         and gate.query in query
                                         and (count or (command == "COMMIT" and admitted))), None)
                            if gate:
                                gate.witness = dict(command=command, rows=count, idle=True,
                                                    receipt_dropped=gate.drop)
                                gate.hit.set()
                    if gate:
                        assert gate.release.wait(4), "proof did not release the withheld receipt"
                        if gate.drop:
                            return
                    cli.sendall(kind + header + data)
                    if grant_rows:
                        admitted = True
                    elif data == b"I":
                        admitted = False
                    query = b""
                    break
                cli.sendall(kind + header + data)
