"""Untimed PG17.11 proof that the exact observer survives a shutdown recycle."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
from tempfile import TemporaryDirectory

from wal_window import decode, preserve, wait_flushed, WAL_STATE

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[4]
sys.path.insert(0, str(HERE.parent))
from wal import reduce


def prove(bindir):
    def invoke(name, *args, text=None, check=True):
        return subprocess.run([bindir / name, *args], input=text, capture_output=True,
                              text=True, check=check, timeout=30)

    with TemporaryDirectory(dir=ROOT / "target", prefix="wr-") as tmp:
        root = Path(tmp)
        data = root / "pg"
        snap = root / "snapshot"
        snap.mkdir()
        invoke("initdb", "-D", data, "-A", "trust", "--no-locale", "-U", "owner", "--wal-segsize=1")
        invoke("pg_ctl", "-D", data, "-l", root / "postgres.log", "-w", "-o",
               f"-c listen_addresses='' -c unix_socket_directories={root}", "start")

        def sql(text):
            return invoke("psql", "-XqAt", "-v", "ON_ERROR_STOP=1", "-h", root, "-U", "owner",
                          "-d", "postgres", text=text).stdout.strip()

        wal = data / "pg_wal"
        try:
            assert sql("SHOW server_version_num") == "170011"
            sql("CREATE TABLE witness (id integer PRIMARY KEY, payload text, state integer) WITH (fillfactor=50)")
            sql("CREATE INDEX witness_state ON witness (state)")
            relation = sql("""SELECT dattablespace || '/' || oid || '/' ||
                        pg_relation_filenode('witness') FROM pg_database WHERE datname=current_database()""")
            start = sql("SELECT pg_current_wal_insert_lsn()")
            sql("INSERT INTO witness VALUES (1, repeat('a',1024), 0)")
            sql("SELECT pg_switch_wal()")
            sql("INSERT INTO witness VALUES (2, repeat('b',1024), 0)")
            sql("UPDATE witness SET state=1 WHERE id=1")
            end = sql("SELECT pg_current_wal_insert_lsn()")
            settings = wait_flushed(lambda: json.loads(sql(WAL_STATE)), end)
            receipt = preserve(wal, snap, start, end, settings)
            assert len(receipt["files"]) == 2
        finally:
            invoke("pg_ctl", "-D", data, "-m", "fast", "-w", "stop")

        control = invoke("pg_controldata", data).stdout
        assert any(line.split(":", 1)[1].strip() == "shut down" for line in control.splitlines()
                   if line.startswith("Database cluster state:"))
        first = receipt["files"][0]
        assert not (wal / first).exists() and (snap / first).is_file()
        original = invoke("pg_waldump", "-p", wal, "-r", "Heap", "-R", relation, "-s", start, "-e", end, check=False)
        assert original.returncode != 0 and first in original.stderr
        captured = decode(bindir, None, snap, root, receipt, relation)
        counts = reduce(captured)
        assert (counts["inserts"], counts["updates"], counts["same_page"], counts["prefix"], counts["hot"]) == (2, 1, 1, 1, 0)
        checkpoint = [line for line in (root / "postgres.log").read_text().splitlines()
                      if "checkpoint complete:" in line][-1]
        activity = re.search(r"(\d+) WAL file\(s\) added, (\d+) removed, (\d+) recycled", checkpoint)
        assert activity and int(activity[2]) + int(activity[3]) > 0, checkpoint.split("; write=")[0]
        return dict(postgres="17.11", range=receipt, relation=relation, first_segment_retired=True, shutdown_removed=int(activity[2]), shutdown_recycled=int(activity[3]),
                    original_exit=original.returncode, original_stderr=original.stderr.strip(),
                    captured=captured.splitlines(), counts=counts, stopped_cleanly=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bindir", type=Path)
    print(json.dumps(prove(parser.parse_args().bindir.resolve()), indent=2))
