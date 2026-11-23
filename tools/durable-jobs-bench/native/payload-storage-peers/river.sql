-- Install only in the disposable River database, as the fixture's parity_owner.
CREATE SCHEMA bench_payload_storage;
REVOKE ALL ON SCHEMA bench_payload_storage FROM PUBLIC;
CREATE TABLE bench_payload_storage.witness (
    schema_name name PRIMARY KEY,
    setup jsonb NOT NULL,
    persisted jsonb,
    verify_start_us bigint,
    verify_end_us bigint
);

CREATE FUNCTION bench_payload_storage.tune_river() RETURNS event_trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp
AS $function$
DECLARE
    cmd record;
    info jsonb;
    ddl text;
    relation text;
    rows_before bigint;
    started bigint;
    finished bigint;
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
        relation := format('%I.river_job', cmd.schema_name);
        SELECT jsonb_build_object(
            'relation', relation, 'relation_oid', c.oid, 'toast_oid', c.reltoastrelid,
            'owner', pg_get_userbyid(c.relowner), 'persistence', c.relpersistence,
            'column', a.attname, 'type', a.atttypid::regtype::text,
            'attstorage', a.attstorage, 'attcompression', a.attcompression)
        INTO STRICT info
        FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid
        WHERE c.oid = cmd.objid AND a.attname = 'args' AND NOT a.attisdropped;
        EXECUTE format('SELECT count(*) FROM %s', relation) INTO rows_before;
        IF info->>'attstorage' <> 'x' OR info->>'type' <> 'jsonb' OR rows_before <> 0 THEN
            RAISE EXCEPTION 'Expected an empty River EXTENDED payload column';
        END IF;

        ddl := format('ALTER TABLE %s ALTER COLUMN args SET STORAGE EXTERNAL', relation);
        started := (extract(epoch FROM clock_timestamp()) * 1000000)::bigint;
        EXECUTE ddl;
        finished := (extract(epoch FROM clock_timestamp()) * 1000000)::bigint;
        IF (SELECT attstorage FROM pg_attribute WHERE attrelid = cmd.objid
            AND attname = 'args' AND NOT attisdropped) <> 'e' THEN
            RAISE EXCEPTION 'River STORAGE EXTERNAL did not take effect';
        END IF;
        DROP EVENT TRIGGER bench_payload_storage_create;
        INSERT INTO bench_payload_storage.witness (schema_name, setup)
        VALUES (cmd.schema_name, jsonb_build_object(
            'before', info, 'after', info || '{"attstorage":"e"}',
            'rows_before', rows_before, 'ddl', ddl, 'ddl_start_us', started, 'ddl_end_us', finished,
            'migration_xid', pg_current_xact_id()::text));
    END LOOP;
END;
$function$;

CREATE FUNCTION bench_payload_storage.snapshot_river() RETURNS event_trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp
AS $function$
DECLARE
    target_schema text;
    persisted_rows jsonb;
    started bigint;
    finished bigint;
BEGIN
    IF current_database() <> 'river' OR session_user <> 'parity_subject'
        OR current_setting('application_name') <> 'river:producer_observer' THEN
        RETURN;
    END IF;

    -- The pinned runner issues this exact cleanup command after terminal
    -- reconciliation. Require the schema recorded by the one-shot create hook.
    target_schema := substring(current_query() FROM '^DROP SCHEMA IF EXISTS (parity_[0-9a-f]{16}) CASCADE$');
    IF NOT EXISTS (SELECT FROM bench_payload_storage.witness WHERE schema_name = target_schema) THEN
        RETURN;
    END IF;
    started := (extract(epoch FROM clock_timestamp()) * 1000000)::bigint;
    EXECUTE format($query$SNAPSHOT_QUERY$query$, target_schema) INTO STRICT persisted_rows;
    DROP EVENT TRIGGER bench_payload_storage_drop;
    finished := (extract(epoch FROM clock_timestamp()) * 1000000)::bigint;
    UPDATE bench_payload_storage.witness SET persisted = persisted_rows,
        verify_start_us = started, verify_end_us = finished WHERE schema_name = target_schema;
END;
$function$;

REVOKE ALL ON FUNCTION bench_payload_storage.tune_river() FROM PUBLIC;
REVOKE ALL ON FUNCTION bench_payload_storage.snapshot_river() FROM PUBLIC;
CREATE EVENT TRIGGER bench_payload_storage_create ON ddl_command_end
    WHEN TAG IN ('CREATE TABLE') EXECUTE FUNCTION bench_payload_storage.tune_river();
CREATE EVENT TRIGGER bench_payload_storage_drop ON ddl_command_start
    WHEN TAG IN ('DROP SCHEMA') EXECUTE FUNCTION bench_payload_storage.snapshot_river();
