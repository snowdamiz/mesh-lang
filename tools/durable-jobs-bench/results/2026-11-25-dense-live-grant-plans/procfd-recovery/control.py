"""Untimed nonroot PostgreSQL 17.11 control with actual TLS connections."""

from contextlib import ExitStack, contextmanager
from copy import deepcopy
import ctypes as C
import json
import os
from pathlib import Path
import select
import socket
import subprocess
import sys
import tempfile

from roster import CAPS, ROSTER, connection_roster, verify_frames


def libpq(path):
    lib = C.CDLL(str(path))
    ptr, string, integer = C.c_void_p, C.c_char_p, C.c_int
    for name, result, args in (
        ("connectdb", ptr, [string]), ("status", integer, [ptr]), ("backendPID", integer, [ptr]),
        ("exec", ptr, [ptr, string]), ("resultStatus", integer, [ptr]),
        ("getvalue", string, [ptr, integer, integer]), ("clear", None, [ptr]), ("finish", None, [ptr]),
    ):
        fn = getattr(lib, "PQ" + name)
        fn.restype, fn.argtypes = result, args
    return lib


@contextmanager
def connect(lib, port, cert, role):
    cfg = f"host=127.0.0.1 port={port} dbname=mesh user={role} sslmode=verify-full sslrootcert={cert}"
    conn = lib.PQconnectdb(cfg.encode())
    try:
        if lib.PQstatus(conn) != 0:
            raise ValueError(f"libpq could not open TLS role {role}")
        yield conn
    finally:
        lib.PQfinish(conn)


def child(lib, port, cert):
    assert os.geteuid() != 0
    if sys.platform == "linux":
        assert C.CDLL(None).prctl(4, 0, 0, 0, 0) == 0  # PR_SET_DUMPABLE
    with ExitStack() as stack:
        roles = {"compiled_caller": 16, "parity_subject": 4}
        opened = {role: [stack.enter_context(connect(lib, port, cert, role)) for _ in range(n)]
                  for role, n in roles.items()}
        print(json.dumps({role: [lib.PQbackendPID(conn) for conn in conns]
                          for role, conns in opened.items()}), flush=True)
        sys.stdin.read()


@contextmanager
def runner(lib_path, port, cert):
    with subprocess.Popen([sys.executable, "-B", __file__, "child", str(lib_path), str(port), str(cert)],
                          stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True) as process:
        try:
            if not select.select([process.stdout], [], [], 15)[0]:
                raise RuntimeError("TLS runner did not bind")
            yield process.pid, json.loads(process.stdout.readline())
        finally:
            process.stdin.close()
            if process.poll() is None:
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
            assert process.returncode == 0


def snapshot(lib, conn):
    result = lib.PQexec(conn, ROSTER.encode())
    try:
        assert lib.PQresultStatus(result) == 2
        return json.loads(lib.PQgetvalue(result, 0, 0))
    finally:
        lib.PQclear(result)


def control(pg, lib_path):
    if os.geteuid() == 0:
        raise ValueError("run as a nonroot user")
    lib = libpq(lib_path)
    with tempfile.TemporaryDirectory(prefix="dense-roster-control.", dir="/tmp") as temp:
        scratch = Path(temp)
        data, cert, key = (scratch / p for p in ("pg", "server.crt", "server.key"))
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            port = s.getsockname()[1]
        subprocess.run(["openssl", "req", "-new", "-x509", "-nodes", "-days", "1", "-newkey", "rsa:2048",
                        "-subj", "/CN=127.0.0.1", "-addext", "subjectAltName=IP:127.0.0.1",
                        "-keyout", key, "-out", cert], check=True, capture_output=True)
        key.chmod(0o600)

        def cmd(tool, *args):
            return subprocess.check_output([pg / tool, *args], text=True, timeout=25).strip()

        cmd("initdb", "-D", data, "-U", "owner", "-A", "trust", "--no-locale")
        with (data / "postgresql.conf").open("a") as stream:
            stream.write(f"\nlisten_addresses='127.0.0.1'\nport={port}\nunix_socket_directories='{scratch}'\n"
                         f"ssl=on\nssl_cert_file='{cert}'\nssl_key_file='{key}'\n"
                         "max_connections=96\nreserved_connections=0\nsuperuser_reserved_connections=0\n")
        (data / "pg_hba.conf").write_text("local all all trust\nhostssl all all 127.0.0.1/32 trust\n")
        cmd("pg_ctl", "-D", data, "-l", scratch / "postgres.log", "-w", "start")
        try:
            cmd("createdb", "-h", scratch, "-p", str(port), "-U", "owner", "mesh")
            cmd("psql", "-XqAt", "-v", "ON_ERROR_STOP=1", "-h", scratch, "-p", str(port),
                "-U", "owner", "mesh", "-c", """
                CREATE ROLE parity_subject LOGIN CONNECTION LIMIT 8;
                CREATE ROLE parity_producer LOGIN CONNECTION LIMIT 1;
                CREATE ROLE compiled_caller LOGIN CONNECTION LIMIT 32;
                GRANT pg_read_all_stats TO parity_producer;
                ALTER DATABASE mesh CONNECTION LIMIT 96;
            """)
            version = cmd("psql", "-XAt", "-h", scratch, "-p", str(port), "-U", "owner", "mesh",
                          "-c", "SHOW server_version_num")
            assert version == "170011"
            with connect(lib, port, cert, "parity_producer") as producer, runner(lib_path, port, cert) as one:
                first = snapshot(lib, producer)
                with runner(lib_path, port, cert) as two:
                    both = snapshot(lib, producer)
                    witness = dict(first=first, both=both, runners_by_index=[one[0], two[0]])
                    actual, ports = connection_roster(witness)
                    for client in (one, two):
                        expected = sorted(actual[pid][1] for pid in client[1]["parity_subject"])
                        assert ports[str(client[0])] == expected
                    assert set(actual) == {lib.PQbackendPID(producer)} | {
                        pid for client in (one, two) for group in client[1].values() for pid in group}
                    frames = [snapshot(lib, producer) for _ in range(3)]
                    frames[0]["runner_ports"] = ports
                    verify_frames(witness, frames)
                    if sys.platform == "linux":
                        for pid in (one[0], two[0]):
                            try:
                                list(Path(f"/proc/{pid}/fd").iterdir())
                            except PermissionError:
                                pass
                            else:
                                raise ValueError("Linux control did not reproduce procfs denial")
        finally:
            cmd("pg_ctl", "-D", data, "-m", "fast", "-w", "stop")

    negatives = {}
    for name, key, value in (
        ("changed_backend_start", "backend_start_us", frames[-1]["sessions"][0]["backend_start_us"] + 1),
        ("changed_backend_pid", "pid", max(actual) + 1),
        ("changed_client_port", "client_port", next(p for p in range(1024,65536)
                                                    if p not in {row[1] for row in actual.values()})),
        ("lost_tls", "ssl", False),
    ):
        changed = deepcopy(frames)
        changed[-1]["sessions"][0][key] = value
        try:
            verify_frames(witness, changed)
        except ValueError:
            negatives[name] = True
        else:
            raise ValueError(f"negative control accepted: {name}")
    print(json.dumps(dict(platform=sys.platform, euid=os.geteuid(), postgres_version_num=version,
                          tls_rosters=[len(first["sessions"]), len(both["sessions"])],
                          participant_connections_by_owned_runner=list(map(len, ports.values())),
                          matched_direct_libpq_backend_pids=True, caps=CAPS, negative_controls=negatives,
                          procfd_denial_reproduced=sys.platform == "linux"), indent=2))


if __name__ == "__main__":
    if sys.argv[1] == "child":
        child(libpq(sys.argv[2]), int(sys.argv[3]), Path(sys.argv[4]))
    else:
        control(Path(sys.argv[1]), Path(sys.argv[2]))
