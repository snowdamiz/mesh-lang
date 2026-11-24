#!/usr/bin/env python3
"""Untimed test of both libpq paths and an independent committed-row observer."""
from contextlib import closing
import json
import os
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory

import measure


def main():
    os.chdir(measure.ROOT)
    sys.path.insert(0, str(measure.SUPPORT.parent))
    import price

    statements, policy, bindings = measure.frozen_inputs()
    parity = price.module(measure.BASE_CLUSTER)
    pg_config = parity.postgres_tools() / "pg_config"
    libdir = subprocess.check_output([pg_config, "--libdir"], text=True).strip()
    libpq = Path(libdir) / ("libpq.dylib" if sys.platform == "darwin" else "libpq.so")
    wire = price.Wire(libpq)
    measure.register_prepared(wire)
    measure.TARGET.mkdir(parents=True, exist_ok=True)
    with TemporaryDirectory(dir=measure.TARGET, prefix="correctness-") as scratch:
        pg = parity.Cluster(Path(scratch), Path(scratch), 96, "17.11")
        try:
            pg.start()
            pg.sql("postgres", f"CREATE ROLE compiled_caller LOGIN PASSWORD '{pg.password}'")
            pg.prepare_database("mesh")
            pg.sql("mesh", policy)
            for named, pair in zip((False, True), bindings[:2], strict=True):
                with closing(price.Session(wire, pg.url("mesh", "compiled_caller"))) as db:
                    assert db.call("""SELECT ssl::text, current_user, current_setting('search_path')
                        FROM pg_stat_ssl WHERE pid=pg_backend_pid()""") == [
                        ("true", "compiled_caller", '"$user", public')]
                    if named:
                        measure.prepare(db, statements)
                    pq, conn = db.pq, db.conn
                    for sql, name, values in zip(statements, measure.NAMES, (pair, pair[:2]), strict=True):
                        assert db.call("SELECT 1") == [("1",)]
                        vals = measure.bind_values(values)
                        if named:
                            result = pq.PQexecPrepared(conn, name, len(vals), vals, None, None, 0)
                        else:
                            result = pq.PQexecParams(conn, sql, len(vals), None, vals, None, None, 0)
                        measure.check_command(db, result, b"INSERT 0 1")
                    prepared = db.call("SELECT name FROM pg_prepared_statements ORDER BY name")
                    assert prepared == ([(n.decode(),) for n in measure.NAMES] if named else [])
            rows = json.loads(pg.sql("mesh", measure.ROWS))
            assert len(measure.verify_rows(rows, bindings[:2], 0)) == 4
        finally:
            pg.stop()
    print("PASS: verify-full PG17.11, named and unnamed writes, four distinct committed XIDs, untimed")


if __name__ == "__main__":
    main()
