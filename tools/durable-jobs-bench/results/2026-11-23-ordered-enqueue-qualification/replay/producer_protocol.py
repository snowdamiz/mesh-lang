#!/usr/bin/env python3
"""Relay and drive the frozen producer session from the original local replay."""
from collections import Counter
import contextlib
import importlib.util
import json
import os
from pathlib import Path
import select
import selectors
import socket
import subprocess
import threading
import time

LANE = Path(__file__).resolve().parent
ROOT = Path(os.environ["ORDERED_SOURCE"])
SPEC = importlib.util.spec_from_file_location("parity", ROOT / "scripts/jobs-parity.py")
parity = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(parity)
PROFILE_HASHES = {
    "dense": "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a",
    "large": "9d7f130f2cc690487582f73c9bea5fbb1f2661f8e416beb2f1574a6f868cc5c1",
    "sparse": "20225faf6274696987b2192136f344d69be243f0a21dc9afad71e714c6ee08c5",
}
NS = "parity_ordered_fixture"

class RecordRelay:
    """Transparent TLS relay used only in separate wire-count runs."""
    def __init__(self, target):
        self.target = target
        self.listener = socket.create_server(("127.0.0.1", 0))
        self.listener.settimeout(10)
        self.port = self.listener.getsockname()[1]
        self.lock = threading.Lock()
        self.counts = Counter()
        self.error = None

    def __enter__(self):
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()
        return self

    def __exit__(self, kind, value, trace):
        self.listener.close()
        self.thread.join(timeout=12)
        if kind is None:
            assert not self.thread.is_alive(), "wire relay did not stop"
            if self.error:
                raise self.error

    def snapshot(self):
        with self.lock:
            return self.counts.copy()

    def run(self):
        try:
            with contextlib.ExitStack() as ctx:
                client, _ = self.listener.accept()
                ctx.enter_context(client)
                server = ctx.enter_context(socket.create_connection(("127.0.0.1", self.target), timeout=10))
                for peer in (client, server):
                    peer.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                    peer.settimeout(10)
                # Postgres SSLRequest and acceptance precede the TLS record stream.
                req = b""
                while len(req) < 8:
                    data = client.recv(8 - len(req))
                    assert data
                    req += data
                assert req.hex() == "0000000804d2162f", req.hex()
                server.sendall(req)
                assert server.recv(1) == b"S"
                client.sendall(b"S")
                poll = ctx.enter_context(selectors.DefaultSelector())
                buffers = {"client": bytearray(), "server": bytearray()}
                poll.register(client, selectors.EVENT_READ, ("client", server))
                poll.register(server, selectors.EVENT_READ, ("server", client))
                while poll.get_map():
                    ready = poll.select(timeout=10)
                    assert ready, "wire relay idle timeout"
                    for key, _ in ready:
                        sender = key.fileobj
                        direction, receiver = key.data
                        data = sender.recv(65536)
                        if not data:
                            poll.unregister(sender)
                            with contextlib.suppress(OSError):
                                receiver.shutdown(socket.SHUT_WR)
                            continue
                        buf = buffers[direction]
                        buf.extend(data)
                        with self.lock:
                            self.counts[f"{direction}_bytes"] += len(data)
                            while len(buf) >= 5:
                                assert buf[0] in (20, 21, 22, 23), buf[:5]
                                size = 5 + int.from_bytes(buf[3:5], "big")
                                if len(buf) < size:
                                    break
                                self.counts[f"{direction}_tls_records"] += 1
                                del buf[:size]
                        receiver.sendall(data)
                assert all(not buf for buf in buffers.values())
        except Exception as error:
            self.error = error

def process_counters(pid):
    filename = Path(f"/proc/{pid}/stat")
    if filename.is_file():
        fields = filename.read_text().rsplit(") ", 1)[1].split()
        return dict(pid=pid, user_ticks=int(fields[11]), kernel_ticks=int(fields[12]),
                    start_ticks=int(fields[19]), affinity=sorted(os.sched_getaffinity(pid)))
    return None


def session(executable, env, requests, out, relay):
    def reply(proc):
        if not select.select([proc.stdout], [], [], 15)[0]:
            raise TimeoutError("frozen producer did not respond")
        line = proc.stdout.readline()
        if not line:
            raise RuntimeError(f"frozen producer exited {proc.poll()}")
        stdout.write(line)
        stdout.flush()
        return line.rstrip("\n")

    receipts = []
    with (out / "producer-stderr.log").open("w") as stderr, (out / "producer-stdout.log").open("w") as stdout:
        proc = subprocess.Popen([executable, "session"], env=env,
                                stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                stderr=stderr, text=True)
        try:
            assert reply(proc) == "SESSION_READY"
            identity = """SELECT json_build_object('user', current_user,
                'tls', (SELECT ssl FROM pg_stat_ssl WHERE pid=pg_backend_pid()),
                'version', current_setting('server_version_num'), 'backend_pid', pg_backend_pid(),
                'durability', ARRAY[current_setting('synchronous_commit'),
                    current_setting('fsync'),current_setting('full_page_writes'),
                    current_setting('track_commit_timestamp')]) AS value"""
            proc.stdin.write(json.dumps({"verb":"query", "sql":identity}) + "\n")
            proc.stdin.flush()
            state = json.loads(reply(proc))
            backend = state["backend_pid"]
            assert state == {"user":"parity_producer", "tls":True, "version":"170011",
                             "durability":["on"] * 4, "backend_pid": backend}, state
            assert isinstance(backend, int) and backend > 1
            parity.write_json(out / "postgres-session.json", state)
            def snapshot():
                data = {label: process_counters(pid) for label, pid in (("producer", proc.pid), ("backend", backend))}
                if data["producer"]:
                    assert all(counters["affinity"] == [0, 1] for counters in data.values())
                return data
            parity.write_json(out / "process-before.json", snapshot())
            for request in requests:
                before = relay.snapshot() if relay else None
                start_ns = time.perf_counter_ns()
                submitted_us = time.time_ns() // 1000
                proc.stdin.write(request)
                proc.stdin.flush()
                raw = reply(proc)
                ack_us = time.time_ns() // 1000
                ack_ms = (time.perf_counter_ns() - start_ns) / 1_000_000
                after = relay.snapshot() if relay else None
                receipts.append(dict(submitted_us=submitted_us, acknowledged_us=ack_us,
                                     ack_ms=ack_ms, response=json.loads(raw),
                                     tls=dict(after - before) if relay else None))
                parity.write_json(out / f"response-{len(receipts):03}.json", receipts[-1])
            parity.write_json(out / "process-after.json", snapshot())
            proc.stdin.write("stop\n")
            proc.stdin.flush()
            assert reply(proc) == "{}"
            assert proc.wait(timeout=5) == 0
        finally:
            if proc.poll() is None:
                proc.kill()
                proc.wait(timeout=5)
    return receipts
