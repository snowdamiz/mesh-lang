#!/usr/bin/env python3
"""Provision one capped PostgreSQL cluster and run a single River matrix arm."""

import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile

from prepare_input import PROFILES, harness

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from verify import PROFILES as MATRIX_PROFILES, reconcile, require
from raw_evidence import SUBMIT_BOUNDARY, frozen_input, verify_run

VERSION = "river v0.51.2 / pgx v5.13.0 / go1.26.4"


class Cluster(harness["Cluster"]):
    def configure_tls(self):
        super().configure_tls()
        with (self.data / "postgresql.conf").open("a") as stream:
            stream.write(f"max_connections = {self.connection_cap}\n"
                         "superuser_reserved_connections = 0\nreserved_connections = 0\n"
                         "fsync = on\nfull_page_writes = on\n")


def arm(args):
    out = args.output.resolve(strict=True)
    require(args.output.is_absolute() and not args.output.is_symlink() and out.is_dir()
            and not any(out.iterdir()), "the matrix must supply an empty absolute output directory")
    binary = args.binary.resolve(strict=True)
    scratch = args.scratch.resolve(strict=True)
    os.environ["PG_BIN_DIR"] = str(args.pg_bin.resolve(strict=True))
    os.umask(0o077)
    raw = out / "river"
    batches, manifest = frozen_input(args.profile)
    binary_hash = harness["digest"](binary)
    phase = os.environ.get("DURABLE_JOBS_PHASE", "diagnostic")
    harness["write_json"](out / "runtime.json", {"phase": phase, "version": VERSION,
                                                 "artifact_sha256": binary_hash, "submit_boundary": SUBMIT_BOUNDARY})
    with tempfile.TemporaryDirectory(prefix="arm-", dir=scratch) as location:
        work = Path(location)
        inputs = work / "input"
        inputs.mkdir()
        paths = harness["batch_files"](inputs, batches)
        require({p.name: harness["digest"](p) for p in paths} == manifest["batch_digests"],
                "generated River batch files differ from the frozen input")
        harness["write_json"](out / "input.json", manifest)
        cluster = Cluster(work, work, MATRIX_PROFILES[args.profile][2], "17.11")
        try:
            cluster.start()
            cluster.sql("postgres", f"CREATE DATABASE river OWNER parity_subject CONNECTION LIMIT {cluster.connection_cap}")
            cluster.sql("postgres", "SELECT pg_stat_statements_reset()")
            env = dict(cluster.env, RIVER_BENCH_DATABASE_URL=cluster.url("river"))
            command = [str(binary), "-single", "-profile", args.profile, "-input", str(inputs), "-out", str(raw)]
            if args.smoke:
                command.append("-smoke")
            result = subprocess.run(command, env=env, check=False)
            if result.returncode:
                raise RuntimeError(f"River binary exited {result.returncode}")
            counters = json.loads(cluster.sql("postgres", """
                SELECT json_build_object('calls', coalesce(sum(calls), 0), 'distinct_statements', count(*),
                    'dealloc', (SELECT dealloc FROM pg_stat_statements_info))
                FROM pg_stat_statements WHERE dbid = (SELECT oid FROM pg_database WHERE datname = 'river')
            """))
            harness["write_json"](out / "counters.json", counters)
            ledger, _ = verify_run(raw, args.profile, binary_hash, smoke=args.smoke)
            require(ledger["version"] == VERSION, "River build differs from the matrix inventory version")
            if args.smoke:
                return
            metrics = reconcile(ledger, "go", args.profile,
                                {"version": VERSION, "artifact_sha256": binary_hash})
            harness["write_json"](out / "ledger.json", ledger)
            harness["write_json"](out / "metrics.json", metrics)
        finally:
            cluster.stop()


def interrupt(signum, _frame):
    raise SystemExit(128 + signum)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--scratch", type=Path, required=True)
    parser.add_argument("--pg-bin", type=Path, required=True)
    parser.add_argument("--smoke", action="store_true", help="functional run with rollback and retry witnesses")
    parser.add_argument("profile", choices=PROFILES)
    parser.add_argument("output", type=Path)
    signal.signal(signal.SIGTERM, interrupt)
    try:
        arm(parser.parse_args())
    except (KeyError, OSError, TypeError, ValueError, RuntimeError) as error:
        raise SystemExit(str(error)) from None


if __name__ == "__main__":
    main()
