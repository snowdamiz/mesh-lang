"""Reuse the frozen disposable PostgreSQL harness with the published worker grants."""

import json
import os
from pathlib import Path
import sys

frozen = Path(os.environ["PRODUCT_FROZEN_HARNESS"]).resolve(strict=True)
sys.path.insert(0, str(frozen / "tools/durable-jobs-bench/native"))
import arm

CLAIM = "mesh_jobs.claim_batch_set_v1(text,text,jsonb,bigint,integer,bigint)"
CURSOR = "mesh_jobs.claim_batch_v1(text,text,jsonb,bigint,integer,bigint)"


def privileges(db):
    return json.loads(db.sql("mesh", f"""
        SELECT json_build_object(
            'worker_set', has_function_privilege('parity_subject', '{CLAIM}', 'EXECUTE'),
            'producer_set', has_function_privilege('parity_producer', '{CLAIM}', 'EXECUTE'),
            'application_set', has_function_privilege('compiled_caller', '{CLAIM}', 'EXECUTE'),
            'worker_cursor', has_function_privilege('parity_subject', '{CURSOR}', 'EXECUTE'),
            'worker_may_insert_job', has_table_privilege('parity_subject', 'mesh_jobs.jobs', 'INSERT'))
    """))


def grant_claim(db):
    before = privileges(db)
    assert before == dict(worker_set=False, producer_set=False, application_set=False,
                          worker_cursor=True, worker_may_insert_job=False), before
    db.sql("mesh", f"GRANT EXECUTE ON FUNCTION {CLAIM} TO parity_subject")
    after = privileges(db)
    assert after == dict(before, worker_set=True), after
    return {"before": before, "after": after}


class Subject(arm.Subject):
    def call(self, action, env, log):
        result = super().call(action, env, log)
        if action == "migrate":
            script = Path(os.environ["PRODUCT_SOURCE"]) / "scripts/fixtures/jobs-role-grants.sql"
            grants = script.read_text().split("-- Optional producer fast path.")[0]
            for role in ("producer", "worker", "operator", "inspector"):
                login = "parity_subject" if role == "worker" else f"parity_{role}"
                grants = grants.replace(f':"{role}"', login)
            self.cluster.sql("mesh", """
                CREATE ROLE parity_migrator NOLOGIN;
                CREATE ROLE parity_operator NOLOGIN;
                CREATE ROLE parity_inspector NOLOGIN;
                REVOKE parity_subject FROM parity_producer;
                REASSIGN OWNED BY parity_subject TO parity_migrator;
                GRANT CONNECT ON DATABASE mesh TO parity_subject, parity_producer, compiled_caller;
                GRANT SELECT ON public.mesh_jobs_parity_attempts, public.mesh_jobs_parity_effects,
                    public.mesh_jobs_parity_product, public.mesh_jobs_parity_authority TO parity_producer;
                GRANT INSERT ON public.mesh_jobs_parity_product TO parity_producer;
            """ + grants)
            self.cluster.evidence.joinpath("privileges.json").write_text(
                json.dumps(grant_claim(self.cluster), indent=2) + "\n")
        return result
