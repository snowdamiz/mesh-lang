SELECT pg_catalog.pg_advisory_xact_lock(213517067, 1);

CREATE TABLE IF NOT EXISTS mesh_jobs.cron_activation (
    namespace text PRIMARY KEY,
    epoch bigint NOT NULL CHECK (epoch > 0),
    activated_at_ms bigint NOT NULL,
    activated_by text NOT NULL,
    seen_at_ms bigint NOT NULL
);

CREATE TABLE IF NOT EXISTS mesh_jobs.cron_schedules (
    namespace text NOT NULL,
    name text NOT NULL,
    id uuid NOT NULL DEFAULT pg_catalog.gen_random_uuid(),
    expression text NOT NULL,
    timezone text NOT NULL,
    misfire text NOT NULL CHECK (misfire IN ('skip', 'coalesce', 'catch_up')),
    lookback_minutes integer NOT NULL CHECK (lookback_minutes BETWEEN 1 AND 1440),
    job jsonb NOT NULL CHECK (jsonb_typeof(job) = 'object'),
    cursor_minute bigint NOT NULL,
    reboot_epoch bigint NOT NULL DEFAULT 0,
    PRIMARY KEY (namespace, name)
);

CREATE INDEX IF NOT EXISTS mesh_jobs_cron_timed
    ON mesh_jobs.cron_schedules (namespace, cursor_minute, name)
    WHERE expression <> '@reboot';

CREATE INDEX IF NOT EXISTS mesh_jobs_cron_reboot
    ON mesh_jobs.cron_schedules (namespace, reboot_epoch, name)
    WHERE expression = '@reboot';
