# Shared helper functions for API modules.
# Provides common utilities used across search, dashboard, and team handlers.
# All functions are pub for cross-module import.

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
