"""Attribute TLS connections from the serial launch boundary."""

from collections import Counter

FIRST = {"compiled_caller": 16, "parity_subject": 4, "parity_producer": 1}
BOTH = {"compiled_caller": 32, "parity_subject": 8, "parity_producer": 1}
CAPS = dict(roles=BOTH, max_connections=96, database_limit=96, reserved=0, superuser_reserved=0)
SESSION_COLUMNS = """p.pid, p.usename, host(p.client_addr) AS client_addr,
                          p.client_port, (extract(epoch FROM p.backend_start)*1000000)::bigint AS backend_start_us,
                          p.state, ssl.ssl"""
ROSTER = f"""
SELECT json_build_object(
    'sessions', (SELECT coalesce(json_agg(row_to_json(s)), '[]') FROM (
        SELECT {SESSION_COLUMNS}
        FROM pg_stat_activity p JOIN pg_stat_ssl ssl USING (pid)
        WHERE p.datname=current_database() AND p.backend_type='client backend') AS s),
    'caps', json_build_object(
        'roles', (SELECT json_object_agg(rolname, rolconnlimit) FROM pg_roles
                  WHERE rolname IN ('parity_subject','parity_producer','compiled_caller')),
        'max_connections', current_setting('max_connections')::int,
        'database_limit', (SELECT datconnlimit FROM pg_database WHERE datname=current_database()),
        'reserved', current_setting('reserved_connections')::int,
        'superuser_reserved', current_setting('superuser_reserved_connections')::int))
"""


def identity(sessions, counts):
    active = [row for row in sessions if row["usename"] in BOTH]
    if Counter(row["usename"] for row in active) != counts:
        raise ValueError("unexpected SQL connection roster")
    keys = {}
    ports = set()
    for row in active:
        pid, port, stamp = (row[k] for k in ("pid", "client_port", "backend_start_us"))
        if (row["ssl"] is not True or row["client_addr"] != "127.0.0.1" or
                type(port) is not int or not 0 < port < 65536 or port in ports or
                type(pid) is not int or pid <= 0 or pid in keys or
                type(stamp) is not int or stamp <= 0):
            raise ValueError("connection identity or TLS changed")
        ports.add(port)
        keys[pid] = (row["usename"], port, stamp)
    return keys


def connection_roster(witness):
    if witness["first"]["caps"] != CAPS or witness["both"]["caps"] != CAPS:
        raise ValueError("PostgreSQL physical connection or role caps changed")
    runners = witness["runners_by_index"]
    if len(runners) != 2 or len(set(runners)) != 2 or any(type(pid) is not int or pid <= 0 for pid in runners):
        raise ValueError("wrong owned runner roster")
    first = identity(witness["first"]["sessions"], FIRST)
    both = identity(witness["both"]["sessions"], BOTH)
    if not first.items() <= both.items():
        raise ValueError("first runner or producer backend replaced during second runner startup")
    second = {pid: both[pid] for pid in both.keys() - first.keys()}
    if Counter(row[0] for row in second.values()) != {"compiled_caller": 16, "parity_subject": 4}:
        raise ValueError("second runner did not own the exact TLS delta")
    ports = {str(runner): sorted(port for role, port, _ in rows.values() if role == "parity_subject")
             for runner, rows in zip(runners, (first, second), strict=True)}
    return both, ports


def verify_frames(witness, frames):
    both, ports = connection_roster(witness)
    if len(frames) != 3 or frames[0]["runner_ports"] != ports:
        raise ValueError("observer frame participant mapping drifted")
    if any(identity(frame["sessions"], BOTH) != both for frame in frames):
        raise ValueError("warmed TLS connection changed across observation frames")
    return both
