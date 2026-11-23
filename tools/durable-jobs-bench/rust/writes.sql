SELECT jsonb_build_object(
    'product', (SELECT coalesce(jsonb_agg(to_jsonb(p) || jsonb_build_object(
        'xid', xmin::text, 'committed_us',
        (extract(epoch FROM pg_xact_commit_timestamp(xmin))*1000000)::bigint)
        ORDER BY request_id), '[]'::jsonb) FROM public.mesh_jobs_parity_product p),
    'attempts', (SELECT coalesce(jsonb_agg(to_jsonb(a) || jsonb_build_object(
        'xid', xmin::text, 'committed_us',
        (extract(epoch FROM pg_xact_commit_timestamp(xmin))*1000000)::bigint)
        ORDER BY job_key, attempt), '[]'::jsonb) FROM public.mesh_jobs_parity_attempts a),
    'effects', (SELECT coalesce(jsonb_agg(to_jsonb(e) || jsonb_build_object(
        'xid', xmin::text, 'committed_us',
        (extract(epoch FROM pg_xact_commit_timestamp(xmin))*1000000)::bigint)
        ORDER BY job_key), '[]'::jsonb) FROM public.mesh_jobs_parity_effects e)
)
