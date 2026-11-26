"""Observe every committed queue state transition in an instrumented short arm."""
import json
import os
from pathlib import Path
import sys

source = Path(os.environ["COMPLETION_WRITER_SOURCE"])
sys.path.insert(0, str(source / "tools/durable-jobs-bench/native"))
import arm

BaseSubject, BaseCluster = arm.Subject, arm.Cluster
TRANSITIONS = """
CREATE TABLE public.writer_transitions (job uuid, prior text, next text);
CREATE FUNCTION public.writer_transition() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
BEGIN
    INSERT INTO public.writer_transitions VALUES (NEW.id, OLD.state, NEW.state);
    RETURN NULL;
END
$$;
CREATE TRIGGER writer_transition AFTER UPDATE OF state ON mesh_jobs.jobs
    FOR EACH ROW WHEN (OLD.state IS DISTINCT FROM NEW.state)
    EXECUTE FUNCTION public.writer_transition();
"""


class DiagnosticSubject(BaseSubject):
    def call(self, action, env, log):
        output = super().call(action, env, log)
        if action == "migrate":
            self.cluster.sql(self.engine, TRANSITIONS)
        return output


class DiagnosticCluster(BaseCluster):
    def reconcile_trial(self, engine, namespace, enqueued_ids, handler_attempts, commits):
        result = super().reconcile_trial(engine, namespace, enqueued_ids, handler_attempts, commits)
        transitions = self.sql(engine, """
            SELECT json_agg(row_to_json(t) ORDER BY committed_us, xid, job) FROM (
              SELECT job, prior, next, xmin::text AS xid,
                    (EXTRACT(EPOCH FROM pg_xact_commit_timestamp(xmin)) * 1000000)::bigint AS committed_us
              FROM public.writer_transitions
            ) t
        """)
        arm.h.write_json(self.evidence / "committed-transitions.json", json.loads(transitions))
        return result


arm.Subject = DiagnosticSubject
arm.Cluster = DiagnosticCluster
arm.main()
