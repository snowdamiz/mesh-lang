"""Local-only adapter for the original product entry point and recovery gate."""
from importlib.util import module_from_spec, spec_from_file_location
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import sys

import contract
import probe


def parity():
    spec = spec_from_file_location("parity", "/checkout/scripts/jobs-parity.py")
    module = module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


h = parity()
# The local image supplies PostgreSQL 17.11 for ARM64. The production
# observer still checks the sealed x86_64 PostgreSQL binary on Linode.
contract.POSTGRES = h.digest(Path(os.environ["PG_BIN_DIR"]) / "postgres")
MODES = {"product-d2": ("success_cohort", True)}
FIXTURE = Path("/fixture")


class Cluster(h.Cluster):
    def configure_tls(self):
        super().configure_tls()
        with (self.data / "postgresql.conf").open("a") as stream:
            stream.write("max_connections = 96\n")


class Subject:
    def __init__(self, cluster):
        self.cluster = cluster

    def call(self, action, env, log):
        assert action == "migrate"
        cluster = self.cluster
        cluster.prepare_database("mesh")
        cluster.sql("mesh", "SET ROLE parity_subject; " + (FIXTURE / "schema.sql").read_text())
        cluster.sql("mesh", """
CREATE ROLE parity_migrator NOLOGIN;
CREATE ROLE parity_operator NOLOGIN;
CREATE ROLE parity_inspector NOLOGIN;
REVOKE parity_subject FROM parity_producer;
REASSIGN OWNED BY parity_subject TO parity_migrator;
GRANT CONNECT ON DATABASE mesh TO parity_subject, parity_producer;
""" + (FIXTURE / "roles.sql").read_text())
        return "migration complete"


def install():
    pass


def install_broker_observer():
    pass


def scan_filters(node, gated=False):
    if node.get("Parent Relationship") in ("InitPlan", "SubPlan"):
        gated = False
    gated = gated or "InitPlan" in node.get("One-Time Filter", "")
    if (node["Node Type"].endswith("Scan") and node.get("Relation Name") == "jobs"
            and node.get("Alias") == "j"):
        yield gated
    for child in node.get("Plans", []):
        yield from scan_filters(child, gated)


def structural_case(sql, declaration, variant, n, record, write):
    for mode in ("force_custom_plan", "force_generic_plan"):
        plan, = json.loads(sql(f"""
BEGIN;
SET LOCAL plan_cache_mode='{mode}';
SET LOCAL ROLE parity_subject;
PREPARE recovery(text,integer,bigint,jsonb) AS {contract.update(declaration)};
EXPLAIN (FORMAT JSON) EXECUTE recovery('{probe.NS}',16,1700000000000,'["default"]');
ROLLBACK;
"""))
        record["plans"][mode] = plan
        write()
        scans = list(scan_filters(plan["Plan"]))
        assert scans, (variant, n, mode)
        if variant == "candidate" and not n:
            assert all(scans), plan


def count_only(sql, admin, db, scenario, due_count):
    # One real recovery per case, under the original worker role. Read only
    # calls and rows. The local proof executes no ANALYZE or timing workload.
    admin(f"SELECT pg_stat_statements_reset(0,(SELECT oid FROM pg_database WHERE datname='{db}'),0)")
    assert sql(probe.SESSION + f"BEGIN; {probe.CALL}; ROLLBACK;") == "0"
    value = json.loads(admin(f"""
SELECT coalesce(json_agg(json_build_array(calls, rows)),'[]')
FROM pg_stat_statements
WHERE dbid=(SELECT oid FROM pg_database WHERE datname='{db}')
AND userid='parity_subject'::regrole AND NOT toplevel AND {probe.UPDATE_FILTER}
"""))
    assert value == [[1, due_count]], (scenario, value)
    return {"calls": 1, "rows": due_count}


def main():
    assert os.geteuid() != 0
    evidence = Path(sys.argv[3])
    assert not list(evidence.iterdir())
    probe.capture_case, probe.measured = structural_case, count_only
    with tempfile.TemporaryDirectory(prefix="er-local-") as scratch:
        cluster = arm.Cluster(Path(scratch), evidence, 96, "17.11")
        try:
            cluster.start()
            arm.Subject(cluster).call("migrate", {}, evidence / "migrate.log")
            gate = json.loads((evidence / "recovery-plan-gate.json").read_bytes())
            fixture = json.loads((evidence / "recovery-fixture.json").read_bytes())
            cases = {}
            for variant, record in gate.items():
                assert record["concurrency"]["two_committed_update_xids"] == 2
                assert record["concurrency"]["statement_trigger_transition_rows"] == 0
                cases[variant] = {}
                for depth, setup in record["fixtures"].items():
                    assert setup["history"]["rows"] == int(depth) and setup["history"]["committed"]
                    cases[variant][depth] = {}
                    for name, proof in setup["cases"].items():
                        assert len(proof["plans"]) == 2 and "after" in proof
                        cases[variant][depth][name] = proof["stats"]["rows"]
            assert cases["stock"] == cases["candidate"]
            assert fixture["phase"] == "gate-stock" and fixture["fixture"]["layout"] and fixture["fixture"]["tls"]
            assert not (evidence / "recovery-plan-gate.pending").exists()
            assert cluster.sql("postgres", "SELECT count(*) FROM pg_database WHERE datname='recovery_pilot'") == "0"
            h.write_json(evidence / "local-proof.json", {
                "uid": os.geteuid(), "server_version": fixture["fixture"]["version"],
                "cases": sum(len(bucket) for variant in cases.values() for bucket in variant.values()),
                "rows": cases["candidate"], "timed": False, "tls": True,
                "layout_guard": True, "clone_dropped": True, "atomic_progress": True,
                "candidate_static_empty_filter": True,
            })
        finally:
            cluster.stop()


arm = SimpleNamespace(main=main, h=h, PROFILES={"noop": (8, 8, 1, 96, None, None)})
