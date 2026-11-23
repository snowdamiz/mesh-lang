SELECT jsonb_build_object(
    'workers', (SELECT coalesce(jsonb_agg(id ORDER BY id), '[]'::jsonb) FROM apalis.workers),
    'sessions', coalesce(jsonb_agg(jsonb_build_object(
        'pid', a.pid, 'application_name', a.application_name, 'ssl', s.ssl, 'user', a.usename,
        'listener', coalesce(a.query = 'LISTEN "apalis::job::insert"', false)
    ) ORDER BY a.application_name, a.pid), '[]'::jsonb)
)
FROM pg_stat_activity a JOIN pg_stat_ssl s USING (pid)
WHERE a.datname = current_database() AND a.backend_type = 'client backend'
