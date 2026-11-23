#!/usr/bin/env python3
"""Cut real PostgreSQL wire replies and inspect the recovered attempts."""
import argparse
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile

from price import FIXTURE, JOBS, digest, module
from wire import Cut, frame

TEST = "jobs::runner::tests::admitted_wire_grant_never_escapes_uncertain_or_malformed_result"


class AdmissionCut(Cut):
    def relay(self):
        super().relay()
        if self.witness and isinstance(self.error, EOFError):
            self.error = None

    def reply(self, pg, cli, target):
        if not target or self.placement in ("partial_row", "drop_ready"):
            return super().reply(pg, cli, target)
        rows = 0
        while True:
            kind, header, data = frame(pg)
            if kind == b"D":
                rows += 1
                if rows == 2:
                    length, = struct.unpack("!i", data[-5:-1])
                    assert length == 1 and data[-1:] == b"2"
                    self.witness = dict(original_ordinal=2, invalid_ordinal=9, rows=rows)
                    if self.placement == "missing_last_row":
                        continue
                    data = data[:-1] + b"9"
            cli.sendall(kind + header + data)
            if kind == b"Z":
                # Continue relaying through the client's ROLLBACK or close.
                assert data == b"T" and rows == 2
                return False


def state(cluster, ns):
    return json.loads(cluster.sql("mesh", f"""
        SELECT json_agg(json_build_array(state,attempt,max_attempts,lease_token IS NOT NULL)
                        ORDER BY position)
        FROM mesh_jobs.jobs WHERE namespace='{ns}'
    """))


def run_case(cluster, executable, placement):
    ns = "wire_" + placement
    cluster.sql("mesh", f"""SET ROLE parity_subject;
        INSERT INTO mesh_jobs.queues(namespace,name,concurrency) VALUES ('{ns}','default',2);
        INSERT INTO mesh_jobs.jobs (namespace,worker,version,queue,args,metadata,tags,state,priority,
            max_attempts,timeout_ms,backoff_base_ms,backoff_max_ms,inserted_at_ms,scheduled_at_ms)
        SELECT '{ns}','parity',1,'default',jsonb_build_object('key',n),'{{}}','[]','available',
            0,n,60000,0,0,0,n FROM generate_series(1,2) n;
    """)
    post_commit = placement == "drop_ready"

    def matches(sql):
        return sql == b"COMMIT" if post_commit else b"FROM mesh_jobs.claim_admitted_v3(" in sql

    with AdmissionCut(cluster.port, matches, placement) as relay:
        env = dict(cluster.env, MESH_JOBS_ADMISSION_TEST_URL=cluster.url("mesh", port=relay.port),
                   MESH_JOBS_ADMISSION_TEST_NAMESPACE=ns)
        fired = subprocess.run([str(executable), "--exact", TEST, "--ignored", "--nocapture"],
                               env=env, text=True, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, timeout=10)
        assert fired.returncode == 0 and "1 passed" in fired.stdout, fired.stdout
    observed = state(cluster, ns)
    expected = [["executing", 1, max_, True] if post_commit else ["available", 0, max_, False]
                for max_ in (1, 2)]
    assert observed == expected, observed
    if post_commit:
        cluster.sql("mesh", f"""SELECT pg_sleep(greatest(0, coalesce(
            (SELECT max(lease_expires_at_ms) FROM mesh_jobs.jobs WHERE namespace='{ns}') -
            mesh_jobs.now_ms() + 5, 0)) / 1000.0);
            SELECT mesh_jobs.recover('{ns}',2,NULL)""")
        after = state(cluster, ns)
        assert after == [["discarded", 1, 1, False], ["retryable", 1, 2, False]], after
    else:
        after = observed
    return dict(rows=observed, after_recovery=after, receipt=relay.witness,
                runtime_returned_launchable_grants=0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("candidate", "scratch", "executable", "output"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    args = parser.parse_args()
    h = module(FIXTURE)
    schema = (args.candidate / JOBS / "schema.sql").read_text()
    with tempfile.TemporaryDirectory(dir=args.scratch, prefix="wire-") as directory:
        work = Path(directory)
        cluster = h.Cluster(work, work, 96)
        try:
            cluster.start()
            assert cluster.sql("postgres", "SHOW server_version_num") == "170011"
            cluster.prepare_database("mesh")
            cluster.sql("mesh", "SET ROLE parity_subject;\n" + schema)
            cases = {placement: run_case(cluster, args.executable.resolve(), placement)
                     for placement in ("partial_row", "bad_ordinal", "missing_last_row", "drop_ready")}
        finally:
            cluster.stop()
    args.output.write_text(json.dumps(dict(schema_sha256=digest(schema.encode()),
                                          test_binary_sha256=digest(args.executable.read_bytes()),
                                          cases=cases), indent=2) + "\n")


if __name__ == "__main__":
    main()
