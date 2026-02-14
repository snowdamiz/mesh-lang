# StorageWriter SQL functions for Mesher monitoring platform.
# Provides the low-level event INSERT using PostgreSQL jsonb extraction.
# All buffer management, retry logic, and service definition live in main.mpl
# because Mesh services and functions with inferred (polymorphic) parameters
# cannot be exported across modules due to type variable scoping limitations.
#
# Events are stored as JSON strings. PostgreSQL parses the JSON server-side
# during INSERT using jsonb extraction operators.

# Insert a single event into the events table from a JSON-encoded string.
# Uses PostgreSQL jsonb extraction to parse event fields server-side.
# Returns the number of rows affected (1 on success).
pub fn insert_event(pool :: PoolHandle, project_id :: String, json_str :: String) -> Int!String do
  let result = Pool.execute(pool, "INSERT INTO events (project_id, issue_id, level, message, fingerprint, exception, stacktrace, breadcrumbs, tags, extra, user_context, sdk_name, sdk_version) SELECT $1::uuid, (j->>'issue_id')::uuid, j->>'level', j->>'message', j->>'fingerprint', (j->'exception')::jsonb, (j->'stacktrace')::jsonb, (j->'breadcrumbs')::jsonb, COALESCE((j->'tags')::jsonb, '{}'::jsonb), COALESCE((j->'extra')::jsonb, '{}'::jsonb), (j->'user_context')::jsonb, j->>'sdk_name', j->>'sdk_version' FROM (SELECT $2::jsonb AS j) AS sub", [project_id, json_str])
  result
end
