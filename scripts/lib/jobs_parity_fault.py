"""One-shot PostgreSQL claim and settlement disconnects for the driver's disposable cluster."""

import selectors
import socket
import struct
import threading
import time
from collections import deque


_LOOPBACK = "127.0.0.1"
_MAX_STARTUP = 10_000
_MAX_FRAME = 4 * 1024 * 1024
_MAX_PENDING = 1024
_MAX_CONNECTIONS = 128
_POLL_SECONDS = 0.05
_IO_SECONDS = 2
_CONFIRM_SECONDS = 10
_LIFETIME_SECONDS = 360

# The last backend message for each request that produces a reply. A Query
# may have more than one CommandComplete. A Sync delimits an extended cycle.
_REPLY_END = {
    b"Q": (b"Z",), b"S": (b"Z",), b"F": (b"Z",),
    b"P": (b"1",), b"B": (b"2",), b"C": (b"3",),
    b"D": (b"T", b"n"), b"E": (b"C", b"I", b"s"),
}


class _InvalidWire(Exception):
    pass


def _cstring(data):
    value, terminator, rest = data.partition(b"\0")
    if not terminator:
        raise _InvalidWire
    return value, rest


class _Frames:
    def __init__(self, startup=False):
        self.startup = startup
        self.buffer = bytearray()

    def feed(self, chunk):
        self.buffer.extend(chunk)
        offset = 0
        while True:
            prefix = 0 if self.startup else 1
            header = prefix + 4
            if len(self.buffer) - offset < header:
                break
            length = struct.unpack_from("!I", self.buffer, offset + prefix)[0]
            if self.startup:
                if length < 9 or length > _MAX_STARTUP:
                    raise _InvalidWire
            elif length < 4 or length > _MAX_FRAME:
                raise _InvalidWire
            end = offset + prefix + length
            if end > len(self.buffer):
                break
            frame = bytes(self.buffer[offset:end])
            offset = end
            if self.startup:
                if frame[4:6] != b"\0\x03" or frame[-1:] != b"\0":
                    raise _InvalidWire
                self.startup = False
                yield b"", b"", frame
            else:
                yield frame[:1], frame[5:], frame
        del self.buffer[:offset]


class _AfterCommit:
    def __init__(self, proxy, matches, *, ready_only):
        self.proxy = proxy
        self.matches = matches
        self.ready_only = ready_only
        # Startup ends in ReadyForQuery, just as a Sync does.
        self.pending = deque([(b"S", False)])
        self.statements = set()
        self.portals = {}
        self.deadline = float("inf")
        self.skip_to_sync = False
        self.held = None
        self.completed = False
        self.failed = False

    def request(self, kind, body):
        matches = False
        if kind == b"Q":
            self.statements.discard(b"")
            self.portals.pop(b"", None)
            matches = self.matches(_cstring(body)[0])
        elif kind == b"P":
            name, rest = _cstring(body)
            self.statements.discard(name)
            if self.matches(_cstring(rest)[0]):
                self.statements.add(name)
        elif kind == b"B":
            portal, rest = _cstring(body)
            statement, _ = _cstring(rest)
            self.portals.pop(portal, None)
            if statement in self.statements:
                self.portals[portal] = statement
        elif kind == b"C":
            name, _ = _cstring(body[1:])
            if body[:1] == b"S":
                self.statements.discard(name)
                self.portals = {portal: statement for portal, statement in self.portals.items()
                                if statement != name}
            elif body[:1] == b"P":
                self.portals.pop(name, None)
        elif kind == b"E":
            matches = _cstring(body)[0] in self.portals

        if kind in _REPLY_END:
            if self.ready_only and matches:
                matches = kind == b"Q" and _cstring(body)[0].strip().upper() == b"COMMIT"
            cut = matches and self.proxy._reserve()
            if cut:
                self.deadline = time.monotonic() + _CONFIRM_SECONDS
            self.pending.append((kind, cut))
        if max(len(self.pending), len(self.statements), len(self.portals)) > _MAX_PENDING:
            raise _InvalidWire

    def response(self, kind, body, frame):
        if self.skip_to_sync and kind != b"Z":
            return self._hold_or_forward(frame), False
        if self.skip_to_sync:
            while self.pending and self.pending[0][0] != b"S":
                self.pending.popleft()
            self.skip_to_sync = False

        if not self.pending:
            return self._hold_or_forward(frame), False
        request, cut = self.pending[0]
        if cut and self.held is None:
            # Postgrex can wait for Describe/Flush before sending Bind/Execute.
            # Follow the matched Execute (or Query) after preparation replies.
            self.held = bytearray()
        if kind == b"E":
            self.failed = self.held is not None
            if request not in (b"Q", b"S", b"F"):
                self.skip_to_sync = True
        if kind == b"C" and cut:
            self.completed = not self.ready_only or body == b"COMMIT\0"
        if kind in _REPLY_END[request]:
            self.pending.popleft()

        output = self._hold_or_forward(frame)
        if kind != b"Z" or self.held is None:
            return output, False
        confirmed = self.completed and not self.failed and body == b"I"
        if not self.ready_only:
            output = self.held
        self.held = None
        self.deadline = float("inf")
        return None if confirmed else output, confirmed

    def _hold_or_forward(self, frame):
        if self.held is None or self.ready_only:
            return frame
        self.held.extend(frame)
        if len(self.held) > _MAX_FRAME:
            raise _InvalidWire
        return None


class SettlementProxy:
    """Relay v3 plaintext sessions to a driver-verified loopback PostgreSQL port.

    Use a fresh instance per probe. The caller verifies cluster ownership before
    entering and supplies a fast predicate over UTF-8 Query or Parse SQL. Read
    ``port`` inside the context. ``fault`` is None until the cut finishes, and
    then holds the placement and protocol observations. The caller checks the
    persisted outcome independently. The after_command_complete placement
    requires a prior claim query and forwards the COMMIT tag while dropping
    the idle ReadyForQuery.
    """

    def __init__(self, upstream_port, sql_matches, placement, *, prior_query=None):
        if type(upstream_port) is not int or not 1 <= upstream_port <= 65535:
            raise ValueError("upstream must be a loopback TCP port")
        if placement not in ("before_commit", "after_commit", "after_command_complete"):
            raise ValueError("invalid fault placement")
        if not callable(sql_matches):
            raise TypeError("sql_matches must be callable")
        if prior_query is not None and not callable(prior_query):
            raise TypeError("prior_query must be callable")
        if placement == "after_command_complete" and prior_query is None:
            raise ValueError("after_command_complete requires a prior claim query")
        self._upstream_port = upstream_port
        self._sql_matches = sql_matches
        self._placement = placement
        self._prior_query = prior_query
        self._claimed = threading.Event()
        self._claim_lock = threading.Lock()
        self._stop = threading.Event()
        self.port = None
        self.fault = None

    def __enter__(self):
        if self.port is not None:
            raise RuntimeError("settlement proxy is single-use")
        listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            listener.bind((_LOOPBACK, 0))
            listener.listen(_MAX_CONNECTIONS)
            listener.settimeout(_POLL_SECONDS)
            self.port = listener.getsockname()[1]
            self._deadline = time.monotonic() + _LIFETIME_SECONDS
            self._thread = threading.Thread(target=self._serve, args=(listener,), daemon=True)
            self._thread.start()
        except BaseException:
            listener.close()
            raise
        return self

    def __exit__(self, *_):
        self._stop.set()
        self._thread.join(2 * _IO_SECONDS + 1)
        if self._thread.is_alive():
            raise RuntimeError("settlement proxy shutdown timed out")

    def _serve(self, listener):
        workers = []
        try:
            with listener:
                while not self._stop.is_set() and time.monotonic() < self._deadline:
                    try:
                        client, _ = listener.accept()
                    except TimeoutError:
                        continue
                    workers = [worker for worker in workers if worker.is_alive()]
                    if len(workers) >= _MAX_CONNECTIONS:
                        client.close()
                        continue
                    worker = threading.Thread(target=self._relay, args=(client,), daemon=True)
                    try:
                        worker.start()
                    except BaseException:
                        client.close()
                        raise
                    workers.append(worker)
        finally:
            self._stop.set()
            for worker in workers:
                worker.join()

    def _matches(self, sql, predicate):
        if self._claimed.is_set():
            return False
        try:
            return predicate(sql.decode("utf-8"))
        except Exception:
            # Predicate or decoding errors must not log SQL or close a session
            # that would otherwise be relayed without a fault.
            return False

    def _reserve(self):
        with self._claim_lock:
            if self._claimed.is_set():
                return False
            self._claimed.set()
            return True

    def _relay(self, client):
        try:
            with client, socket.socket(socket.AF_INET, socket.SOCK_STREAM) as backend:
                client.settimeout(_IO_SECONDS)
                backend.settimeout(_IO_SECONDS)
                backend.connect((_LOOPBACK, self._upstream_port))
                injected = self._bridge(client, backend)
        except (OSError, _InvalidWire):
            return
        if injected:
            confirmed = self._placement != "before_commit"
            fault = {
                "placement": self._placement,
                "query_forwarded": confirmed,
                "command_complete": confirmed,
                "ready_status": "I" if confirmed else None,
            }
            if self._placement == "after_command_complete":
                fault.update(command_complete_forwarded=True, ready_for_query_forwarded=False)
            self.fault = fault

    def _bridge(self, client, backend):
        armed = self._prior_query is None

        def matches(sql):
            nonlocal armed
            if not armed:
                armed = self._matches(sql, self._prior_query)
                return False
            return self._matches(sql, self._sql_matches)

        after = None
        if self._placement != "before_commit":
            after = _AfterCommit(self, matches, ready_only=self._placement == "after_command_complete")
        with selectors.DefaultSelector() as selector:
            selector.register(client, selectors.EVENT_READ, (backend, _Frames(startup=True)))
            selector.register(backend, selectors.EVENT_READ, (client, _Frames()))
            while not self._stop.is_set():
                deadline = min(self._deadline, after.deadline) if after else self._deadline
                if time.monotonic() >= deadline:
                    break
                for key, _ in selector.select(_POLL_SECONDS):
                    dest, parser = key.data
                    chunk = key.fileobj.recv(64 * 1024)
                    if not chunk:
                        return False
                    for kind, body, frame in parser.feed(chunk):
                        if self._stop.is_set():
                            return False
                        if key.fileobj is client:
                            if after:
                                after.request(kind, body)
                            elif self._drop_query(kind, body, matches):
                                return True
                        elif after:
                            frame, confirmed = after.response(kind, body, frame)
                            if confirmed:
                                return True
                        if frame:
                            dest.sendall(frame)
        return False

    def _drop_query(self, kind, body, matches):
        if kind not in (b"P", b"Q") or self._claimed.is_set():
            return False
        if kind == b"P":
            _, body = _cstring(body)
        return matches(_cstring(body)[0]) and self._reserve()
