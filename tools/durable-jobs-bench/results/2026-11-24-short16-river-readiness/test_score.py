"""Untimed checks of the startup receipt contract."""

from io import BytesIO
import json
import tarfile
import unittest

from score import EMPTY, RIVER, startup


def proc(pid):
    return dict(pid=pid, start_ticks=100, user_ticks=2, system_ticks=1, threads=1, rss_bytes=4096)


def backend(pid):
    return dict(pid=pid, backend_start="session", application_name="river:producer_observer" if pid == 100 else
                "river:parity_1111111111111111:runner-0", db_user="parity_subject",
                database="river", superuser=False, tls=True, observer=pid == 100, last_query_listen=pid == 101)


def fixture():
    points = []
    for name, time in (("before_runner", 1_000_000), ("ready", 1_100_000),
                       ("idle_middle", 2_600_000), ("before_submit", 4_100_000)):
        pids = [100] if name == "before_runner" else range(100, 118)
        points.append(dict(point=name, epoch_us=time, elapsed_us=time - 1_000_000,
                           rows=EMPTY.copy(), backends=[backend(pid) for pid in pids],
                           postgres_backends=[proc(pid) for pid in pids], producer=proc(10), runner=proc(11)))
    return {
        RIVER + "startup.json": dict(mode="warm", producer_pid=10, runner_pid=11, points=points),
        RIVER + "pool-fill.json": dict(started_us=1_010_000, finished_us=1_055_000, elapsed_us=45_000,
                                       pool_max=18, backend_pids=list(range(101, 118))),
        "adapter/river/producer-startup.json": dict(elapsed_us=500, before=proc(10), after=proc(10)),
        "adapter/river/config.json": dict(runner_pool_max=18, producer_observer_pool_max=1, poll_only=False,
                                          poll_ms=250, fetch_cooldown_ms=5, idle_ms=3000,
                                          profile_config=dict(runners=1, slots=16)),
        "adapter/river/database.json": dict(max_connections=20, database_connection_limit=20, user="parity_subject",
                                            reserved_connections=0, superuser_reserved_connections=0,
                                            other_client_backends=0),
    }


def replay(doc):
    buffer = BytesIO()
    with tarfile.open(fileobj=buffer, mode="w") as archive:
        for name, data in doc.items():
            contents = json.dumps(data).encode()
            member = tarfile.TarInfo(name)
            member.size = len(contents)
            archive.addfile(member, BytesIO(contents))
    buffer.seek(0)
    with tarfile.open(fileobj=buffer) as archive:
        return startup(archive, "warm", dict(first_submit_us=4_100_999), 100)


class StartupTest(unittest.TestCase):
    def test_retained_pool_and_listener_reconcile(self):
        self.assertEqual(replay(fixture())["physical_at_submit"], 18)

    def test_reject_unstable_session(self):
        data = fixture()
        data[RIVER + "startup.json"]["points"][-1]["backends"][-1]["backend_start"] = "reopened"
        with self.assertRaisesRegex(ValueError, "retain all physical sessions"):
            replay(data)

    def test_reject_pre_submit_writes_and_missing_tls(self):
        for kind in ("rows", "tls", "listener"):
            with self.subTest(kind=kind):
                data = fixture()
                point = data[RIVER + "startup.json"]["points"][-1]
                if kind == "rows":
                    point["rows"]["effects"] = 1
                elif kind == "tls":
                    point["backends"][1]["tls"] = False
                else:
                    point["backends"][1]["last_query_listen"] = False
                with self.assertRaises(ValueError):
                    replay(data)
