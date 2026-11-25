"""Prove per-runner TLS ownership from serial launches and stable SQL identities."""

from collections import Counter

ROSTER = """
SELECT json_build_object(
    'sessions', (SELECT coalesce(json_agg(row_to_json(s) ORDER BY s.pid), '[]') FROM (
        SELECT p.pid, p.usename AS role, host(p.client_addr) AS addr, p.client_port,
               (extract(epoch FROM p.backend_start)*1000000)::bigint AS backend_start_us,
               ssl.ssl AS tls
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


def roles(runners, started):
    session_counts = {"parity_producer": 1}
    if started:
        session_counts.update(compiled_caller=(8 // runners) * started, parity_subject=4 * started)
    return session_counts


def snapshot(frame, runners, started):
    if runners not in (1, 2) or not 0 <= started <= runners:
        raise ValueError("wrong runner shape")
    caps = dict(roles=roles(runners, runners), max_connections=68, database_limit=68,
                reserved=0, superuser_reserved=0)
    if frame["caps"] != caps:
        raise ValueError("PostgreSQL physical or role caps changed")
    sessions = frame["sessions"]
    if Counter(row["role"] for row in sessions) != roles(runners, started):
        raise ValueError("unexpected SQL connection roster")
    identities, ports = {}, set()
    for row in sessions:
        pid, port, stamp = (row[k] for k in ("pid", "client_port", "backend_start_us"))
        if (row["tls"] is not True or row["addr"] != "127.0.0.1"
                or type(pid) is not int or pid <= 0 or pid in identities
                or type(port) is not int or not 0 < port < 65536 or port in ports
                or type(stamp) is not int or stamp <= 0):
            raise ValueError("TLS or connection identity changed")
        identities[pid] = (row["role"], port, stamp)
        ports.add(port)
    return identities


def verify(witness, pids):
    runners = len(pids)
    if witness["runner_pids"] != pids or len(set(pids)) != runners or any(
            type(pid) is not int or pid <= 0 for pid in pids):
        raise ValueError("wrong launched runner roster")
    prefixes = witness["prefixes"]
    if len(prefixes) != runners:
        raise ValueError("missing serial launch boundary")
    ready = witness["ready"]
    before = witness["before_submit"]
    frames = [*prefixes, ready, before]
    if any(a["observed_us"] >= b["observed_us"] for a, b in zip(frames, frames[1:])):
        raise ValueError("ready or submit clock moved backwards")
    previous = snapshot(prefixes[0], runners, 0)
    owners = {}
    for index, (pid, frame) in enumerate(zip(pids, [*prefixes[1:], ready], strict=True), 1):
        current = snapshot(frame, runners, index)
        if not previous.items() <= current.items():
            raise ValueError("an existing backend changed across a runner launch")
        delta = {backend: current[backend] for backend in current.keys() - previous.keys()}
        if Counter(row[0] for row in delta.values()) != {"compiled_caller": 8 // runners, "parity_subject": 4}:
            raise ValueError("runner did not add the exact application and participant TLS sessions")
        owners[str(pid)] = [dict(backend=backend, client_port=port)
                            for backend, (role, port, _) in sorted(delta.items())
                            if role == "compiled_caller"]
        previous = current
    if snapshot(before, runners, runners) != previous:
        raise ValueError("a warmed backend changed before submit")
    return owners


def standard(frame):
    return [dict(role=row["role"], backend=row["pid"], client_port=row["client_port"], tls=row["tls"])
            for row in frame["sessions"]]
