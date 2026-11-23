SELECT jsonb_build_object(
    'connections', (SELECT count(*) FROM pg_stat_activity WHERE datname = current_database()),
    'all_tls', (SELECT bool_and(s.ssl) FROM pg_stat_activity a JOIN pg_stat_ssl s USING (pid)
                WHERE a.datname = current_database() AND a.backend_type = 'client backend'),
    'executing', count(*) FILTER (WHERE status = 'Running'),
    'queued', count(*) FILTER (WHERE status = 'Queued'),
    'terminal', count(*) FILTER (WHERE status IN ('Done', 'Killed')
                                    OR (status = 'Failed' AND attempts >= max_attempts)),
    'failed', count(*) FILTER (WHERE status = 'Killed'
                                  OR (status = 'Failed' AND attempts >= max_attempts)),
    'total', count(*)
)
FROM apalis.jobs
