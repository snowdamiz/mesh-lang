"""Diagnostic loopback PG/SCRAM bridge. Each verified TLS leg uses the fixture CA."""
import contextlib
import hashlib
import socket
import ssl
import struct
import threading


def read_exact(peer, length):
    result = bytearray()
    while len(result) != length:
        part = peer.recv(min(length - len(result), 16384))
        if not part:
            raise EOFError(f"received {len(result)}/{length} PostgreSQL bytes")
        result.extend(part)
    return result


def forward(peer, frame):
    # Deliberately split every header and relay the body in bounded writes.
    peer.sendall(frame[:2])
    peer.sendall(frame[2:5])
    for start in range(5, len(frame), 16384):
        peer.sendall(frame[start:start + 16384])


def packet(peer):
    head = read_exact(peer, 5)
    length = struct.unpack_from("!I", head, 1)[0]
    assert 4 <= length <= 64 * 1024 * 1024
    return bytes(head + read_exact(peer, length - 4))


def bind(body):
    assert body[0:2] == b"\0\0"
    formats = struct.unpack_from("!H", body, 2)[0]
    index = 4 + 2 * formats
    count = struct.unpack_from("!H", body, index)[0]
    param_formats = struct.unpack_from(f"!{formats}h", body, 4)
    index += 2
    lengths = []
    args = []
    for _ in range(count):
        length = struct.unpack_from("!i", body, index)[0]
        assert length >= -1
        lengths.append(length)
        index += 4
        args.append(body[index:index + max(0, length)])
        index += max(0, length)
    results = struct.unpack_from("!H", body, index)[0]
    result_formats = struct.unpack_from(f"!{results}h", body, index + 2)
    assert index + 2 + 2 * results == len(body)
    return dict(lengths=lengths, param_formats=param_formats, result_formats=result_formats), args


class Bridge(contextlib.AbstractContextManager):
    def __init__(self, cluster, spec_path=None):
        self.cluster = cluster
        self.spec_path = spec_path
        self.transcript = []
        self.error = None
        self.listener = socket.create_server(("127.0.0.1", 0))
        self.listener.settimeout(30)
        self.port = self.listener.getsockname()[1]

    def __enter__(self):
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()
        return self

    def __exit__(self, kind, _, __):
        self.listener.close()
        self.thread.join(45)
        if self.error:
            if kind is None:
                raise self.error
            raise RuntimeError(f"diagnostic bridge failed: {self.error!r}") from self.error
        assert kind is not None or not self.thread.is_alive(), "diagnostic bridge hung"

    def run(self):
        try:
            self.relay()
        except Exception as error:
            self.error = error

    def relay(self):
        request = bytes.fromhex("0000000804d2162f")
        upstream_tls = ssl.create_default_context(cafile=str(self.cluster.certificate))
        # The harness CA omits an Authority Key Identifier, which Python 3.13+
        # rejects under its strict default. Chain and hostname stay verified.
        upstream_tls.verify_flags &= ~ssl.VERIFY_X509_STRICT
        downstream_tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        downstream_tls.load_cert_chain(self.cluster.data / "server.crt", self.cluster.data / "server.key")
        with contextlib.ExitStack() as sockets:
            raw_client, _ = self.listener.accept()
            sockets.enter_context(raw_client)
            raw_client.settimeout(45)
            raw_server = sockets.enter_context(socket.create_connection(("127.0.0.1", self.cluster.port), timeout=45))
            assert read_exact(raw_client, 8) == request
            raw_server.sendall(request)
            assert read_exact(raw_server, 1) == b"S"
            raw_client.sendall(b"S")
            client = sockets.enter_context(downstream_tls.wrap_socket(raw_client, server_side=True))
            server = sockets.enter_context(upstream_tls.wrap_socket(raw_server, server_hostname="127.0.0.1"))
            for peer in (client, server):
                peer.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            self.tls = [client.version(), server.version()]
            size = read_exact(client, 4)
            forward(server, size + read_exact(client, struct.unpack("!I", size)[0] - 4))
            while True:
                response = packet(server)
                forward(client, response)
                if response[0] == ord("R") and struct.unpack_from("!I", response, 5)[0] in (10, 11):
                    auth = packet(client)
                    assert auth[0] == ord("p")
                    forward(server, auth)
                if response[0] == ord("Z"):
                    break
            sql = ""
            while True:
                exchange = []
                while True:
                    frame = packet(client)
                    tag = chr(frame[0])
                    item = dict(tag=tag, bytes=len(frame), sha256=hashlib.sha256(frame).hexdigest())
                    if tag == "P":
                        assert frame[5] == 0
                        sql = frame[6:].split(b"\0", 1)[0].decode()
                        item["sql"] = sql
                    if tag == "Q":
                        item["sql"] = frame[5:-1].decode()
                    if tag == "B":
                        item["bind"], args = bind(frame[5:])
                        if self.spec_path and sql.startswith("SELECT mesh_jobs.enqueue(") and not self.spec_path.exists():
                            # The synthetic first admission's real four values feed the micro harness.
                            for number, arg in enumerate(args):
                                self.spec_path.with_suffix(f".{number}").write_bytes(arg)
                            self.spec_path.touch()
                    exchange.append(item)
                    forward(server, frame)
                    if tag in "QSX":
                        break
                self.transcript.append(exchange)
                if tag == "X":
                    return
                while True:
                    response = packet(server)
                    forward(client, response)
                    if response[0] == ord("Z"):
                        break
