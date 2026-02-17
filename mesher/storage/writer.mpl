# StorageWriter SQL functions for Mesher monitoring platform.
# Provides the low-level event INSERT using PostgreSQL jsonb extraction.
# All buffer management, retry logic, and service definition live in main.mpl
# because Mesh services and functions with inferred (polymorphic) parameters
# cannot be exported across modules due to type variable scoping limitations.
#
# Events are stored as JSON strings. PostgreSQL parses the JSON server-side
# during INSERT using jsonb extraction operators.
# issue_id and fingerprint are passed as separate SQL parameters (not extracted
# from JSON) -- see research Open Question 1, Option B.

# Insert a single event into the events table from a JSON-encoded string.
# issue_id and fingerprint are passed separately (computed by EventProcessor
# via extract_event_fields + upsert_issue) rather than extracted from JSON.
# Uses PostgreSQL jsonb extraction for remaining fields.
# Returns the number of rows affected (1 on success).
pub fn insert_event(pool :: PoolHandle, project_id :: String, issue_id :: String, fingerprint :: String, json_str :: String) -> Int!String do
  let result = Repo.execute_raw(pool, "INSERT INTO events (project_id, issue_id, level, message, fingerprint, exception, stacktrace, breadcrumbs, tags, extra, user_context, sdk_name, sdk_version) SELECT $1::uuid, $2::uuid, j->>'level', j->>'message', $3, (j->'exception')::jsonb, (j->'stacktrace')::jsonb, (j->'breadcrumbs')::jsonb, COALESCE((j->'tags')::jsonb, '{}'::jsonb), COALESCE((j->'extra')::jsonb, '{}'::jsonb), (j->'user_context')::jsonb, j->>'sdk_name', j->>'sdk_version' FROM (SELECT $4::jsonb AS j) AS sub", [project_id, issue_id, fingerprint, json_str])
  result
end
