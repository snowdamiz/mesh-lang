"""Scripted response cuts for a single connection to the owned SCRAM loopback fixture."""
import socket
import struct
from threading import Thread


def read(sock, count):
    data = bytearray()
    while len(data) < count:
        chunk = sock.recv(count - len(data))
        if not chunk:
            raise EOFError("fixture connection closed")
        data.extend(chunk)
    return bytes(data)


def frame(sock):
    kind = read(sock, 1)
    header = read(sock, 4)
    length = struct.unpack("!I", header)[0]
    assert 4 <= length <= 17 * 1024**2
    return kind, header, read(sock, length - 4)


class Cut:
    def __init__(self, upstream, matches, placement):
        self.upstream = upstream
        self.matches = matches
        self.placement = placement
        self.witness = None
        self.error = None

    def __enter__(self):
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(1)
        self.listener.settimeout(15)
        self.port = self.listener.getsockname()[1]
        self.thread = Thread(target=self.relay)
        self.thread.start()
        return self

    def __exit__(self, exc, *_):
        self.thread.join(18)
        self.listener.close()
        assert not self.thread.is_alive(), "cut relay did not close"
        if exc is None:
            if self.error:
                raise self.error
            assert self.witness, "cut did not occur"

    def relay(self):
        try:
            cli, _ = self.listener.accept()
            with cli, socket.create_connection(("127.0.0.1", self.upstream), timeout=15) as pg:
                cli.settimeout(15)
                header = read(cli, 4)
                size = struct.unpack("!I", header)[0]
                assert 8 <= size <= 10_000
                startup = read(cli, size - 4)
                assert startup.startswith(b"\0\3\0\0")
                pg.sendall(header + startup)
                while True:
                    kind, header, data = frame(pg)
                    cli.sendall(kind + header + data)
                    if kind == b"R" and struct.unpack("!I", data[:4])[0] in (10, 11):
                        auth = frame(cli)
                        assert auth[0] == b"p"
                        pg.sendall(b"".join(auth))
                    if kind == b"Z":
                        break

                target = False
                while True:
                    kind, header, data = frame(cli)
                    if kind == b"P":
                        target = self.matches(data.split(b"\0", 2)[1])
                    if kind == b"Q":
                        target = self.matches(data[:-1])
                        if target and self.placement == "before_request":
                            self.witness = {"forwarded": False}
                            return
                    pg.sendall(kind + header + data)
                    if kind in (b"Q", b"S"):
                        if self.reply(pg, cli, target):
                            return
                        target = False
        except BaseException as error:
            self.error = error

    def reply(self, pg, cli, target):
        command = None
        rows = 0
        while True:
            kind, header, data = frame(pg)
            if kind == b"D":
                rows += 1
                if target and self.placement == "partial_row":
                    prefix = max(1, len(data) // 3)
                    cli.sendall(kind + header + data[:prefix])
                    self.witness = {"forwarded": True, "row_index": rows, "row_bytes": len(data),
                                    "forwarded_row_bytes": prefix, "complete_reply": False}
                    return True
            if kind == b"C":
                command = data[:-1].decode()
            if kind == b"Z":
                if target and self.placement == "drop_ready":
                    expected = {"COMMIT": (b"I", 0), "FETCH 8": (b"I", 8), "SELECT 8": (b"T", 8)}
                    assert (data, rows) == expected[command]
                    self.witness = {"forwarded": True, "result_rows_forwarded": rows,
                                    "command_complete_forwarded": command,
                                    "ready_state_received_from_server": data.decode(),
                                    "ready_forwarded": False}
                    return True
                cli.sendall(kind + header + data)
                return False
            cli.sendall(kind + header + data)
