SELECT jsonb_build_object(
    'runner', coalesce(sum(calls) FILTER (WHERE rolname = 'parity_subject'), 0),
    'producer_observer', coalesce(sum(calls) FILTER (WHERE rolname = 'parity_producer'), 0),
    'application', coalesce(sum(calls) FILTER (WHERE rolname = 'parity_subject' AND
        query ~ 'INSERT INTO public.mesh_jobs_parity_(attempts|effects)'), 0),
    'claims', coalesce(sum(calls) FILTER (WHERE rolname = 'parity_subject' AND
        query LIKE '%FROM%apalis.get_jobs%'), 0),
    'dealloc', (SELECT dealloc FROM pg_stat_statements_info)
)
FROM pg_stat_statements JOIN pg_roles ON userid = pg_roles.oid
WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database())
