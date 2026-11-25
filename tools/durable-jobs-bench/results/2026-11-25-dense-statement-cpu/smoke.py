"""Private PostgreSQL correctness canary with real backend and postmaster exits."""
import json
from pathlib import Path
import socket
import subprocess
import sys
import tempfile

from measure import read_report


def canary(binaries, module, work, enabled):
    data = work / "pgdata"
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = str(sock.getsockname()[1])
    log = work / "postgres.log"

    def pg(tool, *args, **kwargs):
        return subprocess.run([binaries / tool, *args], check=True, text=True, timeout=20, **kwargs)

    pg("initdb", "-D", data, "-U", "cpu_smoke", "--no-locale", "--auth=trust", stdout=subprocess.DEVNULL)
    with (data / "postgresql.conf").open("a") as conf:
        conf.write(f"""
listen_addresses = '127.0.0.1'
port = {port}
unix_socket_directories = ''
shared_preload_libraries = 'pg_stat_statements,{module}'
dense_stmt_cpu.enabled = {'on' if enabled else 'off'}
pg_stat_statements.track = 'all'
""")
    try:
        pg("pg_ctl", "-D", data, "-l", log, "-w", "start", stdout=subprocess.DEVNULL)
    except subprocess.CalledProcessError:
        print(log.read_text(), file=sys.stderr)
        raise
    try:
        result = pg("psql", "-XAtq", "-h", "127.0.0.1", "-p", port, "-U", "cpu_smoke", "-d", "postgres",
                    input=r"""
CREATE EXTENSION pg_stat_statements;
SHOW server_version_num;
SHOW dense_stmt_cpu.enabled;
SELECT 'pid=' || pg_backend_pid();
CREATE TABLE cpu_probe (n int);
INSERT INTO cpu_probe VALUES (1), (2);
CREATE FUNCTION cpu_nested() RETURNS bigint LANGUAGE plpgsql VOLATILE AS $$
DECLARE answer bigint;
BEGIN
  BEGIN PERFORM 1/n FROM (VALUES (0)) AS bad(n); EXCEPTION WHEN division_by_zero THEN NULL; END;
  SELECT sum(n) INTO answer FROM cpu_probe;
  RETURN answer;
END $$;
SELECT cpu_nested();
SELECT 'key=' || queryid FROM pg_stat_statements WHERE toplevel AND query = 'SELECT cpu_nested()';
SELECT 'inner=' || queryid FROM pg_stat_statements WHERE NOT toplevel AND query LIKE 'SELECT sum(n) %cpu_probe';
BEGIN;
SELECT n/(n-n) FROM cpu_probe;
ROLLBACK;
SELECT pg_stat_statements_reset();
BEGIN;
DECLARE cpu_cursor NO SCROLL CURSOR FOR SELECT n FROM cpu_probe;
FETCH 1 FROM cpu_cursor;
\! sleep 1
FETCH 1 FROM cpu_cursor;
CLOSE cpu_cursor;
COMMIT;
SELECT 'after=' || sum(n) FROM cpu_probe;
SELECT 'after_key=' || queryid FROM pg_stat_statements WHERE toplevel AND query LIKE '%sum(n) FROM cpu_probe' AND query NOT LIKE '%pg_stat_statements%';
""", capture_output=True)
        outputs = result.stdout.splitlines()
        assert outputs[:2] == ["170011", "on" if enabled else "off"], outputs
        assert "3" in outputs and "after=3" in outputs and "1" in outputs and "2" in outputs, outputs
        assert result.stderr.count("ERROR:") == 1 and "division by zero" in result.stderr, result.stderr
        assert not (data / "dense-cpu.jsonl").exists(), "file appeared while postmaster was running"
        keys = dict(line.split("=", 1) for line in outputs if "=" in line)
    finally:
        pg("pg_ctl", "-D", data, "-m", "fast", "-w", "stop", stdout=subprocess.DEVNULL)
    header, sessions, counts = read_report((data / "dense-cpu.jsonl").read_text(), enabled)
    assert len(sessions) == 1 and sessions[0]["pid"] == int(keys["pid"])
    entries = {row["query"]: row for row in counts}
    if enabled:
        assert sessions[0]["nested"] > 0 and keys["inner"] not in entries
        assert entries[keys["key"]]["cpu_ns"] > 0, "reset erased CPU counters"
        assert entries[keys["after_key"]]["calls"][1] == 1
        assert sum(row["errors"] for row in counts) == 1, "caught SPI error escaped or top-level error was lost"
        assert sum(row["cpu_ns"] for row in counts) < 500_000_000, "client idle leaked into CPU"
    else:
        assert not counts and not sessions[0]["nested"]
    return dict(enabled=enabled, postmaster=header["postmaster"], sessions=len(sessions),
                tested=["preload", "backend-exit", "off-clock-publish", "session-cpu"]
                + (["nested-spi", "caught-error", "abort", "pgss-reset", "client-idle"] if enabled else []))


if __name__ == "__main__":
    pg, so, root = [Path(arg).resolve(strict=True) for arg in sys.argv[1:]]
    with tempfile.TemporaryDirectory(prefix="smoke-", dir=root) as temp:
        result = []
        for enabled in (False, True):
            folder = Path(temp) / str(int(enabled))
            folder.mkdir()
            result.append(canary(pg, so, folder, enabled))
    print(json.dumps(result))
