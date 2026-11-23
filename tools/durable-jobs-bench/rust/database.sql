SELECT jsonb_build_object(
    'version', current_setting('server_version_num')::int / 10000 || '.' ||
        current_setting('server_version_num')::int % 10000,
    'tls', (SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()),
    'fsync', current_setting('fsync') = 'on',
    'synchronous_commit', current_setting('synchronous_commit') = 'on',
    'full_page_writes', current_setting('full_page_writes') = 'on',
    'track_commit_timestamp', current_setting('track_commit_timestamp') = 'on',
    'max_connections', current_setting('max_connections')::integer,
    'database_connection_limit', (SELECT datconnlimit FROM pg_database WHERE datname = current_database()),
    'reserved_connections', current_setting('reserved_connections')::integer,
    'superuser_reserved_connections', current_setting('superuser_reserved_connections')::integer
)
