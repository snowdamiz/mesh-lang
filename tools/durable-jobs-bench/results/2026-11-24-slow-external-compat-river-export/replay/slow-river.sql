-- Use only the disposable River database. The migration hook reads metadata
-- in the actual producer session. The cleanup hook reads the settled rows.
CREATE SCHEMA bench_slow_storage;
REVOKE ALL ON SCHEMA bench_slow_storage FROM PUBLIC;
CREATE TABLE bench_slow_storage.witness (
    schema_name name PRIMARY KEY,
    setup jsonb NOT NULL,
    persisted jsonb,
    verify_start_us bigint,
    verify_end_us bigint
);

CREATE FUNCTION bench_slow_storage.capture_layout() RETURNS event_trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp
AS $function$
DECLARE
    cmd record;
    metadata jsonb;
    empty_table boolean;
    external_ddl text;
    original_metadata jsonb;
BEGIN
    IF current_database() <> 'river' OR session_user <> 'parity_subject'
        OR current_setting('application_name') <> 'river:producer_observer' THEN
        RETURN;
    END IF;
    FOR cmd IN
        SELECT d.objid, d.schema_name
        FROM pg_event_trigger_ddl_commands() d
        JOIN pg_class c ON d.classid = 'pg_class'::regclass AND c.oid = d.objid
        WHERE d.command_tag = 'CREATE TABLE' AND NOT d.in_extension AND d.objsubid = 0
          AND d.schema_name ~ '^parity_[0-9a-f]{16}$'
          AND c.relname = 'river_job' AND c.relkind = 'r' AND c.relpersistence = 'p'
          AND c.relowner = session_user::regrole
    LOOP
        SELECT jsonb_build_object(
            'relation', format('%I.river_job', cmd.schema_name),
            'relation_oid', c.oid, 'toast_oid', c.reltoastrelid, 'persistence', c.relpersistence,
            'column', a.attname, 'type', a.atttypid::regtype::text,
            'attstorage', a.attstorage,
            'attcompression', CASE a.attcompression WHEN 'p' THEN 'pglz'
                                                WHEN 'l' THEN 'lz4' ELSE 'default' END,
            'default_toast_compression', current_setting('default_toast_compression'))
        INTO STRICT metadata
        FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid
        WHERE c.oid = cmd.objid AND a.attname = 'args' AND NOT a.attisdropped;
        EXECUTE format('SELECT NOT EXISTS (SELECT FROM %I.river_job)', cmd.schema_name) INTO empty_table;
        IF NOT empty_table OR metadata->>'default_toast_compression' <> 'pglz'
            OR metadata->>'attstorage' <> 'x' OR metadata->>'attcompression' <> 'default' THEN
            RAISE EXCEPTION 'River layout or producer policy differs from the selected arm';
        END IF;
        original_metadata := metadata;
        IF APPLY_EXTERNAL THEN
            external_ddl := format('ALTER TABLE %I.river_job ALTER COLUMN args SET STORAGE EXTERNAL', cmd.schema_name);
            EXECUTE external_ddl;
            IF (SELECT attstorage FROM pg_attribute
                WHERE attrelid = cmd.objid AND attname = 'args' AND NOT attisdropped) <> 'e' THEN
                RAISE EXCEPTION 'River payload column did not become EXTERNAL';
            END IF;
            metadata := metadata || '{"attstorage":"e"}'::jsonb;
        END IF;
        INSERT INTO bench_slow_storage.witness VALUES (cmd.schema_name,
            jsonb_build_object('before', original_metadata, 'metadata', metadata, 'rows', 0, 'ddl', external_ddl,
                'producer_session_policy', current_setting('default_toast_compression'),
                'checked_us', (extract(epoch FROM clock_timestamp()) * 1000000)::bigint));
        DROP EVENT TRIGGER bench_slow_storage_create;
    END LOOP;
END;
$function$;

CREATE FUNCTION bench_slow_storage.capture_rows() RETURNS event_trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp
AS $function$
DECLARE
    target_schema text;
    snapshot jsonb;
    started bigint;
BEGIN
    IF current_database() <> 'river' OR session_user <> 'parity_subject'
        OR current_setting('application_name') <> 'river:producer_observer' THEN
        RETURN;
    END IF;
    target_schema := substring(current_query() FROM '^DROP SCHEMA IF EXISTS (parity_[0-9a-f]{16}) CASCADE$');
    IF NOT EXISTS (SELECT FROM bench_slow_storage.witness WHERE schema_name = target_schema) THEN
        RETURN;
    END IF;
    started := (extract(epoch FROM clock_timestamp()) * 1000000)::bigint;
    EXECUTE format($query$SNAPSHOT_QUERY$query$, target_schema) INTO STRICT snapshot;
    UPDATE bench_slow_storage.witness SET persisted = snapshot, verify_start_us = started,
        verify_end_us = (extract(epoch FROM clock_timestamp()) * 1000000)::bigint
    WHERE schema_name = target_schema;
    DROP EVENT TRIGGER bench_slow_storage_drop;
END;
$function$;

REVOKE ALL ON FUNCTION bench_slow_storage.capture_layout() FROM PUBLIC;
REVOKE ALL ON FUNCTION bench_slow_storage.capture_rows() FROM PUBLIC;
CREATE EVENT TRIGGER bench_slow_storage_create ON ddl_command_end
    WHEN TAG IN ('CREATE TABLE') EXECUTE FUNCTION bench_slow_storage.capture_layout();
CREATE EVENT TRIGGER bench_slow_storage_drop ON ddl_command_start
    WHEN TAG IN ('DROP SCHEMA') EXECUTE FUNCTION bench_slow_storage.capture_rows();
