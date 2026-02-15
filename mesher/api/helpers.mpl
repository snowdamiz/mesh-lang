# Shared helper functions for API modules.
# Provides common utilities used across search, dashboard, and team handlers.
# All functions are pub for cross-module import.

# Cluster-aware registry lookup.
# Tries node-local Process.whereis first (zero overhead).
# Falls back to Global.whereis for cross-node discovery in cluster mode.
# In standalone mode, Process.whereis always succeeds (Global.whereis never called).
pub fn get_registry() do
  let local = Process.whereis("mesher_registry")
  if local != 0 do
    local
  else
    Global.whereis("mesher_registry")
  end
end

# Extract optional query parameter with a default value.
# Request.query returns Option<String>; case match to Some/None.
pub fn query_or_default(request, param :: String, default :: String) -> String do
  let opt = Request.query(request, param)
  case opt do
    Some(v) -> v
    None -> default
  end
end

# Extract a required path parameter.
# Request.param returns Option<String>; route matching guarantees existence.
pub fn require_param(request, name :: String) -> String do
  let opt = Request.param(request, name)
  case opt do
    Some(v) -> v
    None -> ""
  end
end

# Convert a list of JSON strings to a JSON array.
# Replaces the old recursive json_array_loop pattern with String.join.
pub fn to_json_array(items) -> String do
  "[" <> String.join(items, ",") <> "]"
end
