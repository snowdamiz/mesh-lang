"""Exercise the recorder and fail-closed replay on real PostgreSQL 17.11 with untimed fixture writes."""
from copy import deepcopy
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
from tempfile import TemporaryDirectory
from types import SimpleNamespace

import controller
import wal_capture as wal

ROOT = Path(__file__).resolve().parents[4]
TARGET = ROOT / "target/native-dense-wal-cost"
TABLE = "mesh_jobs_parity_attempts"


def read(path):
    return json.loads(path.read_text())


def expect_bad(before, after, closure, score, edit, reason):
    inputs = deepcopy([before, after, closure, score])
    edit(*inputs)
    try:
        controller.verify_wal(*inputs)
    except ValueError as error:
        assert reason in str(error), (error, reason)
    else:
        raise AssertionError(f"missing rejection: {reason}")


def main(pg_bin):
    os.environ["PG_BIN_DIR"] = str(pg_bin)
    harness = wal.load("dense_wal_proof_base", ROOT / "scripts/jobs-parity.py")

    class Cluster(harness.Cluster):
        def configure_tls(self):
            super().configure_tls()
            with (self.data / "postgresql.conf").open("a") as stream:
                stream.write("max_connections = 96\nreserved_connections = 0\n"
                             "superuser_reserved_connections = 0\nwal_sync_method = fdatasync\n")

    with TemporaryDirectory(dir=TARGET, prefix="pg17-proof-") as directory:
        root = Path(directory)
        fixture = SimpleNamespace(Cluster=Cluster)
        wal.install(fixture, root, "postgres")
        cluster = fixture.Cluster(root, root, 96, "17.11")
        active = None
        try:
            cluster.start()
            psql, env = cluster.psql("postgres")
            cluster.sql("postgres", f"CREATE TABLE {TABLE} (id int PRIMARY KEY)")
            cluster.sql("postgres", wal.RESET)
            first_submit = wal.stamp()["epoch_us"]
            active = subprocess.Popen(psql, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
            xids = []
            for i in (1, 2):
                active.stdin.write(f"INSERT INTO {TABLE} VALUES ({i}) RETURNING xmin;\n")
                active.stdin.flush()
                xids.append(int(active.stdout.readline()))
            assert len(set(xids)) == 2
            last_settled = wal.stamp()["epoch_us"]
            assert cluster.sql("postgres", f"SELECT count(*) FROM {TABLE}") == "2"
            during = json.loads(cluster.sql("postgres", "SELECT row_to_json(w) FROM pg_stat_wal w"))
            try:
                wal.collect(cluster)
            except ValueError as error:
                assert "overlapped workload clients" in str(error)
            else:
                raise AssertionError("observer failed to reject an open workload client")
            active.stdin.close()
            assert active.wait(timeout=15) == 0
            active.stdout.close()
            active = None
            cluster.stop()
        finally:
            if active:
                active.terminate()
                active.wait(timeout=15)
            # Basic cleanup also covers failed startup and failed readback.
            Cluster.stop(cluster)
            assert not (cluster.data / "postmaster.pid").exists()

        before, after, closure = (read(root / f"wal-{name}.json")
                                  for name in ("before", "live-after", "restored"))
        score = dict(first_submit_us=first_submit, last_settlement_us=last_settled)
        result = controller.verify_wal(before, after, closure, score)
        assert result["scope"].startswith("whole workload")
        live = after["frame"]["wal"]
        yield_on_disconnect = live["wal_sync"] - during["wal_sync"]
        assert yield_on_disconnect > 0, "live pre-exit snapshot did not expose delayed WAL publication"
        assert any(s["kind"] == "attempt" and s["rows"] == s["calls"] == 2 and s["wal_bytes"] > 0
                   for s in result["statement_wal"]["rows"])

        expect_bad(before, after, closure, score,
                   lambda b, a, c, s: s.update(first_submit_us=b["begin"]["epoch_us"]),
                   "overlapped benchmark clocks")
        expect_bad(before, after, closure, score,
                   lambda b, a, c, s: c["restored"]["frame"]["wal"].update(stats_reset="mutated"),
                   "reset or lost")
        expect_bad(before, after, closure, score,
                   lambda b, a, c, s: a["frame"].update(other_clients=1),
                   "settings, TLS, clients, or clocks differ")
        expect_bad(before, after, closure, score,
                   lambda b, a, c, s: c["restored"]["frame"]["wal"].update(wal_sync=0),
                   "nonmonotonic")
        print(json.dumps(dict(
            purpose="untimed native PostgreSQL behavior proof; two fixture rows",
            system=platform.system(), machine=platform.machine(),
            postgres=subprocess.check_output([pg_bin / "postgres", "--version"], text=True).strip(),
            frozen_config=wal.SETTINGS, independent_commits=len(xids), delayed_syncs_published=yield_on_disconnect,
            clean_restart_retained_stats=True,
            before_recorded_before_writes=True, client_socket_guard=True, whole_workload_fallback=True,
            invalid_evidence_rejections=4,
            capture_sha256=hashlib.sha256(Path(wal.__file__).read_bytes()).hexdigest(),
            scorer_sha256=hashlib.sha256(Path(controller.__file__).read_bytes()).hexdigest()),
            indent=2, sort_keys=True))


if __name__ == "__main__":
    main(Path(sys.argv[1]))
