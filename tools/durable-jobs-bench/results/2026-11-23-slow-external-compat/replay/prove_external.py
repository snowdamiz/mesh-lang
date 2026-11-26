#!/usr/bin/env python3
"""Replay the original compiled proof on owner-approved EXTERNAL and exercise catalog mutations."""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import types

import layout
import storage

HERE = Path(__file__).resolve().parent
FROZEN = HERE.parent / "frozen"
PROOF = FROZEN / "prove-original.py"


def proof_module(lane):
    source_root = lane / "source/scripts"
    spec = importlib.util.spec_from_file_location("jobs_parity", source_root / "jobs-parity.py")
    parity = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(parity)
    sys.path.insert(0, str(source_root))
    support = types.ModuleType("screen")
    support.LANE, support.MODE, support.parity, support.json_out = (
        lane, "MESH_JOBS_SET_BASED_ENQUEUE", parity, storage.write
    )

    def scope(cluster, db):
        cluster.sql(db, """
            CREATE ROLE bulk_worker;
            CREATE ROLE bulk_operator;
            CREATE ROLE bulk_inspector;
            REVOKE parity_subject FROM parity_producer;
            GRANT INSERT ON public.mesh_jobs_parity_product TO parity_producer;
        """)
        cmd, env = cluster.psql(db)
        subprocess.run([*cmd, "-v", "producer=parity_producer", "-v", "worker=bulk_worker",
                        "-v", "operator=bulk_operator", "-v", "inspector=bulk_inspector",
                        "-f", str(source_root / "fixtures/jobs-role-grants.sql")],
                       env=env, check=True, timeout=10)

    support.scope = scope
    sys.modules["screen"] = support
    original = PROOF.read_text()
    if storage.sha(original.encode()) != "ae496eeabe3dddea4a8263d6ad80be9d6a702147d03f414e52dc4b4f5cef7756":
        raise ValueError("frozen original batch proof changed")
    before = '''("storage","ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTERNAL",
             "ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTENDED")'''
    after = '''("storage","ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE MAIN",
             "ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTERNAL")'''
    if original.count(before) != 1:
        raise ValueError("original layout control changed")
    proof = types.ModuleType("compiled_proof")
    exec(compile(original.replace(before, after), PROOF, "exec"), proof.__dict__)
    return proof


def verify_writer(cluster, db, rows, bulk, scalar, writes_wal):
    observation = json.loads(cluster.sql(db, """
        SELECT coalesce(jsonb_agg(to_jsonb(w) ORDER BY query), '[]') FROM (
            SELECT query, calls, rows, wal_bytes FROM pg_stat_statements
            WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database())
              AND userid = 'parity_producer'::regrole
              AND (query LIKE 'SELECT ordinal, receipt FROM mesh_jobs.enqueue_batch_v1(%'
                OR query LIKE 'SELECT mesh_jobs.enqueue(%')
        ) w
    """))
    batch = [line for line in observation if line["query"].startswith("SELECT ordinal, receipt")]
    ordered = [line for line in observation if line["query"].startswith("SELECT mesh_jobs.enqueue(")]
    assert len(batch) == int(bool(bulk)) and len(ordered) == int(bool(scalar)), observation
    if batch:
        entry, = batch
        assert (entry["calls"], entry["rows"], entry["wal_bytes"] > 0) == (bulk, rows, writes_wal), observation
    if ordered:
        entry, = ordered
        assert entry["calls"] == entry["rows"] == scalar, observation
    return observation


def extra_cases(proof, cluster, out):
    mutations = {
        "args-plain": "ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE PLAIN",
        "args-compression-pglz": "ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET COMPRESSION pglz",
        "other-jobs-column": "ALTER TABLE mesh_jobs.jobs ALTER COLUMN metadata SET STORAGE EXTERNAL",
        "other-table-column": "ALTER TABLE mesh_jobs.receipts ALTER COLUMN receipt SET STORAGE EXTERNAL",
        "args-type": "CREATE DOMAIN public.layout_args AS jsonb; "
                     "ALTER TABLE mesh_jobs.jobs ALTER COLUMN args TYPE public.layout_args "
                     "USING args::public.layout_args; "
                     "ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTERNAL",
        "new-column": "ALTER TABLE mesh_jobs.jobs ADD COLUMN layout_mutation integer",
        "new-index": "CREATE INDEX layout_mutation ON mesh_jobs.jobs (worker)",
        "new-constraint": "ALTER TABLE mesh_jobs.jobs ADD CONSTRAINT layout_mutation CHECK (worker <> '')",
        "toast-option": "ALTER TABLE mesh_jobs.jobs SET (toast_tuple_target = 2560)",
        "force-rls": "ALTER TABLE mesh_jobs.jobs FORCE ROW LEVEL SECURITY",
        "effect-key-body": None,
        "claim-trigger-body": None,
        "wake-trigger-body": None,
    }
    functions = {
        "effect-key-body": "mesh_jobs.effect_key(mesh_jobs.jobs)",
        "claim-trigger-body": "mesh_jobs.fence_claim_v1()",
        "wake-trigger-body": "mesh_jobs.publish_wake_v1()",
    }
    for label, ddl in mutations.items():
        with proof.trial(cluster, out, f"mutation-{label}", "bulk_qualified") as (db, directory):
            proof.check_layout(cluster, db, "t")
            if ddl is None:
                name = functions[label]
                definition = cluster.sql(db, f"SELECT pg_get_functiondef('{name}'::regprocedure)")
                anchor = "AS $function$"
                assert definition.count(anchor) == 1
                ddl = definition.replace(anchor, anchor + "\n/* catalog mutation control */")
            cluster.sql(db, "SET ROLE parity_subject; " + ddl)
            proof.check_layout(cluster, db, "f")
            receipts = proof.invoke(cluster, db, directory, label, proof.specs(label))
            saved = proof.db_rows(cluster, db, label)
            assert len(receipts) == len(saved["rows"]) == saved["receipts"] == proof.CASES
            assert [r["id"] for r in receipts] == [r["id"] for r in saved["rows"]]
            storage.write(directory / "writer.json", verify_writer(cluster, db, 1, 1, proof.CASES, False))
            storage.write(directory / "ledger.json", saved)

    for label, physical in (("default", False), ("external", True)):
        with proof.trial(cluster, out, f"admitted-{label}", "bulk_qualified") as (db, directory):
            if not physical:
                cluster.sql(db, "SET ROLE parity_subject; ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTENDED")
            proof.check_layout(cluster, db, "t")
            rows = proof.specs(label)
            for row in rows:
                row["args"]["payload"] = "abcd" * 4096
            receipts = proof.invoke(cluster, db, directory, label, rows)
            meta = json.loads(cluster.sql(db, """
                SELECT jsonb_agg(jsonb_build_object('compression', pg_column_compression(args),
                        'bytes', pg_column_size(args))) FROM mesh_jobs.jobs
            """))
            assert len(receipts) == len(meta) == proof.CASES
            assert {entry["compression"] for entry in meta} == ({None} if physical else {"pglz"})
            assert all((entry["bytes"] >= 16384) == physical for entry in meta)
            storage.write(directory / "physical.json", meta)
            storage.write(directory / "writer.json", verify_writer(cluster, db, proof.CASES, 1, 0, True))

    with proof.trial(cluster, out, "producer-cannot-change-layout", "bulk_qualified") as (db, directory):
        blocked = []
        for sql in ("ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE MAIN",
                    layout.INSTALL.read_text()):
            try:
                cluster.sql(db, "SET ROLE parity_producer; " + sql)
            except proof.parity.FixtureError as error:
                assert "must be owner" in str(error) or "permission denied" in str(error), error
                blocked.append(str(error))
            else:
                raise ValueError("scoped producer changed owner-only DDL")
        proof.check_layout(cluster, db, "t")
        storage.write(directory / "denials.json", blocked)


def run(lane, out):
    proof = proof_module(lane)
    out.mkdir()
    with tempfile.TemporaryDirectory(dir=lane / "tmp", prefix="external-proof-") as path:
        cluster = proof.parity.Cluster(Path(path), out, 16)
        try:
            cluster.start()
            version = int(cluster.sql("postgres", "SHOW server_version_num"))
            if version != 170011:
                raise ValueError(f"expected PostgreSQL 17.11, received {version}")
            proof.setup(cluster, out)
            witness = layout.install(cluster, "bulk_qualified", external=True, compatible=True)
            layout.check_install(witness, external=True, compatible=True)
            cluster.sql("bulk_template", """
                SET ROLE parity_subject;
                ALTER TABLE mesh_jobs.jobs ALTER COLUMN args SET STORAGE EXTERNAL
            """)
            storage.write(out / "manifest.json", dict(version=version, installer=witness,
                artifacts={p.name: proof.parity.digest(p) for p in (proof.OLD, proof.NEW)}))
            proof.cases(cluster, out, legacy_only=False)
            extra_cases(proof, cluster, out)
            (out / "PASS").write_text("EXTERNAL compiled protocol, rollback, layout and mixed legacy receipts reconciled\n")
        finally:
            try:
                cluster.stop()
            finally:
                for path in out.rglob("*"):
                    if path.is_file():
                        data = path.read_bytes()
                        if cluster.password.encode() in data:
                            path.write_bytes(data.replace(cluster.password.encode(), b"<redacted>"))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("work", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    run(args.work.resolve(), args.output.resolve())
