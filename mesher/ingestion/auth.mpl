# Authentication helpers for API key extraction and project lookup.
# Used by both middleware and route handlers for DSN-style auth.

from Storage.Queries import get_project_id_by_key

# Try the Authorization header as fallback.
fn try_authorization_header(request) -> Option<String> do
  let bearer = Request.header(request, "authorization")
  case bearer do
    Some(token) -> Some(token)
    None -> None
  end
end

# Extract API key from request headers.
# Checks X-Sentry-Auth header first, falls back to Authorization header.
# Returns Option<String> with the raw key value.
pub fn extract_api_key(request) -> Option<String> do
  let auth = Request.header(request, "x-sentry-auth")
  case auth do
    Some(key) -> Some(key)
    None -> try_authorization_header(request)
  end
end

# Authenticate a request by looking up the API key against the database.
# Returns the project ID string if the key is valid and non-revoked.
# Uses String!String return type to avoid struct-in-Result ABI issues
# (Result<Struct, String> causes segfault due to mismatched sum type layout).
pub fn authenticate_request(pool :: PoolHandle, request) -> String!String do
  let key_opt = extract_api_key(request)
  case key_opt do
    Some(key) -> get_project_id_by_key(pool, key)
    None -> Err("missing API key")
  end
end
