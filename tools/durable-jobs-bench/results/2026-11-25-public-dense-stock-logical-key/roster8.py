"""Bind each owned launch to its incremental warm PostgreSQL TLS roster."""

from roster import identity

RUNNERS = 8


def counts(runners):
    return dict(compiled_caller=4*runners, parity_subject=4*runners, parity_producer=1)


CAPS = dict(roles=counts(RUNNERS), max_connections=96, database_limit=96,
            reserved=0, superuser_reserved=0)


def bind(witness, frames=()):
    snapshots = witness["snapshots"]
    runners = witness["runners_by_index"]
    assert len(snapshots) == 9 and len(runners) == len(set(runners)) == RUNNERS
    assert all(type(pid) is int and pid > 0 for pid in runners)
    assert all(snapshot["caps"] == CAPS for snapshot in snapshots)
    previous = identity(snapshots[0]["sessions"], {"parity_producer": 1})
    ports = {}
    for index, (runner, frame) in enumerate(zip(runners, snapshots[1:], strict=True), 1):
        current = identity(frame["sessions"], counts(index))
        assert previous.items() <= current.items(), "prior runner TLS identity changed"
        delta = [current[pid] for pid in current.keys() - previous.keys()]
        ports[str(runner)] = sorted(port for role, port, _ in delta if role == "parity_subject")
        assert len(ports[str(runner)]) == 4
        previous = current
    if frames:
        assert len(frames) == 3 and frames[0]["runner_ports"] == ports
        assert all(identity(frame["sessions"], counts(RUNNERS)) == previous for frame in frames)
    return previous, ports
