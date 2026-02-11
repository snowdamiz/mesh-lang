//! HTTP router for the Snow runtime.
//!
//! Routes are checked with priority ordering: exact routes first, then
//! parameterized routes (`:param` segments), then wildcards. Within each
//! tier, first match wins (registration order).
//!
//! Supports:
//! - Exact match: `/api/health` matches `/api/health`
//! - Wildcard patterns: `/api/*` matches any path starting with `/api/`
//! - Path parameters: `/users/:id` matches `/users/42` and captures `id=42`
//! - Method-specific routing: `HTTP.on_get(r, "/path", handler)` matches only GET

use crate::string::SnowString;

/// A single route entry mapping a URL pattern to a handler.
pub struct RouteEntry {
    /// URL pattern (exact, wildcard ending with `/*`, or parameterized with `:name`).
    pub pattern: String,
    /// Optional HTTP method filter. None = any method, Some("GET") = only GET.
    pub method: Option<String>,
    /// Pointer to the handler function.
    pub handler_fn: *mut u8,
    /// Pointer to the handler closure environment (null for bare functions).
    pub handler_env: *mut u8,
}

/// Router holding an ordered list of route entries.
pub struct SnowRouter {
    pub routes: Vec<RouteEntry>,
}

/// Check if a pattern has any parameterized segments (`:name`).
fn has_param_segments(pattern: &str) -> bool {
    pattern.split('/').any(|seg| seg.starts_with(':'))
}

/// Segment-based matching with parameter extraction.
///
/// Splits both pattern and path on `/`, filters empty segments, then compares
/// pairwise. Literal segments must match exactly; `:name` segments capture the
/// actual value into the returned params vec.
///
/// Returns `Some(params)` on match, `None` on mismatch.
fn match_segments(pattern: &str, path: &str) -> Option<Vec<(String, String)>> {
    let pat_segs: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let path_segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if pat_segs.len() != path_segs.len() {
        return None;
    }
    let mut params = Vec::new();
    for (pat, actual) in pat_segs.iter().zip(path_segs.iter()) {
        if pat.starts_with(':') {
            params.push((pat[1..].to_string(), actual.to_string()));
        } else if pat != actual {
            return None;
        }
    }
    Some(params)
}

/// Check if a pattern matches a path (for exact and wildcard routes only).
///
/// - Exact match: "/api/health" matches "/api/health"
/// - Wildcard: "/api/*" matches "/api/users", "/api/users/123", etc.
/// - Root wildcard: "/*" matches everything
fn matches_pattern(pattern: &str, path: &str) -> bool {
    if pattern.ends_with("/*") {
        let prefix = &pattern[..pattern.len() - 1]; // strip the '*', keep the '/'
        path.starts_with(prefix) || path == &pattern[..pattern.len() - 2]
    } else {
        pattern == path
    }
}

/// Check if a pattern is a wildcard (ends with `/*`).
fn is_wildcard(pattern: &str) -> bool {
    pattern.ends_with("/*")
}

impl SnowRouter {
    /// Find the first route matching the given path and HTTP method.
    ///
    /// Returns (handler_fn, handler_env, params) or None.
    /// Uses three-pass matching for priority:
    ///   1. Exact routes (no `:param`, no `*`) -- highest priority
    ///   2. Parameterized routes (`:param` segments) -- medium priority
    ///   3. Wildcard routes (`/*`) -- lowest priority (catch-all fallback)
    /// Within each pass, also checks method filtering.
    pub fn match_route(&self, path: &str, method: &str) -> Option<(*mut u8, *mut u8, Vec<(String, String)>)> {
        // First pass: exact routes only (no `:param` segments, no wildcards).
        for entry in &self.routes {
            if has_param_segments(&entry.pattern) || is_wildcard(&entry.pattern) {
                continue;
            }
            if let Some(ref m) = entry.method {
                if m != method {
                    continue;
                }
            }
            if matches_pattern(&entry.pattern, path) {
                return Some((entry.handler_fn, entry.handler_env, Vec::new()));
            }
        }

        // Second pass: parameterized routes (have `:param` segments).
        for entry in &self.routes {
            if !has_param_segments(&entry.pattern) {
                continue;
            }
            if let Some(ref m) = entry.method {
                if m != method {
                    continue;
                }
            }
            if let Some(params) = match_segments(&entry.pattern, path) {
                return Some((entry.handler_fn, entry.handler_env, params));
            }
        }

        // Third pass: wildcard routes (catch-all fallback).
        for entry in &self.routes {
            if !is_wildcard(&entry.pattern) {
                continue;
            }
            if let Some(ref m) = entry.method {
                if m != method {
                    continue;
                }
            }
            if matches_pattern(&entry.pattern, path) {
                return Some((entry.handler_fn, entry.handler_env, Vec::new()));
            }
        }

        None
    }
}

// ── Internal helper ──────────────────────────────────────────────────

/// Add a route with an optional method filter to the router.
/// Returns a NEW router pointer (immutable semantics).
fn route_with_method(
    router: *mut u8,
    pattern: *const SnowString,
    handler_fn: *mut u8,
    method: Option<&str>,
) -> *mut u8 {
    let handler_env: *mut u8 = std::ptr::null_mut();
    unsafe {
        let old = &*(router as *const SnowRouter);
        let pat_str = (*pattern).as_str().to_string();

        let mut new_routes = Vec::with_capacity(old.routes.len() + 1);
        for entry in &old.routes {
            new_routes.push(RouteEntry {
                pattern: entry.pattern.clone(),
                method: entry.method.clone(),
                handler_fn: entry.handler_fn,
                handler_env: entry.handler_env,
            });
        }
        new_routes.push(RouteEntry {
            pattern: pat_str,
            method: method.map(|m| m.to_string()),
            handler_fn,
            handler_env,
        });

        let new_router = Box::new(SnowRouter {
            routes: new_routes,
        });
        Box::into_raw(new_router) as *mut u8
    }
}

// ── Public extern "C" API ──────────────────────────────────────────────

/// Create an empty router. Returns a pointer to a heap-allocated SnowRouter.
#[no_mangle]
pub extern "C" fn snow_http_router() -> *mut u8 {
    let router = Box::new(SnowRouter {
        routes: Vec::new(),
    });
    Box::into_raw(router) as *mut u8
}

/// Add a route to the router (method-agnostic). Returns a NEW router pointer.
///
/// This is the existing `HTTP.route(router, pattern, handler)` -- matches
/// any HTTP method (backward compatible).
#[no_mangle]
pub extern "C" fn snow_http_route(
    router: *mut u8,
    pattern: *const SnowString,
    handler_fn: *mut u8,
) -> *mut u8 {
    route_with_method(router, pattern, handler_fn, None)
}

/// Add a GET-only route. Returns a NEW router pointer.
#[no_mangle]
pub extern "C" fn snow_http_route_get(
    router: *mut u8,
    pattern: *const SnowString,
    handler_fn: *mut u8,
) -> *mut u8 {
    route_with_method(router, pattern, handler_fn, Some("GET"))
}

/// Add a POST-only route. Returns a NEW router pointer.
#[no_mangle]
pub extern "C" fn snow_http_route_post(
    router: *mut u8,
    pattern: *const SnowString,
    handler_fn: *mut u8,
) -> *mut u8 {
    route_with_method(router, pattern, handler_fn, Some("POST"))
}

/// Add a PUT-only route. Returns a NEW router pointer.
#[no_mangle]
pub extern "C" fn snow_http_route_put(
    router: *mut u8,
    pattern: *const SnowString,
    handler_fn: *mut u8,
) -> *mut u8 {
    route_with_method(router, pattern, handler_fn, Some("PUT"))
}

/// Add a DELETE-only route. Returns a NEW router pointer.
#[no_mangle]
pub extern "C" fn snow_http_route_delete(
    router: *mut u8,
    pattern: *const SnowString,
    handler_fn: *mut u8,
) -> *mut u8 {
    route_with_method(router, pattern, handler_fn, Some("DELETE"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::snow_rt_init;
    use crate::string::snow_string_new;

    #[test]
    fn test_exact_match() {
        assert!(matches_pattern("/api/health", "/api/health"));
        assert!(!matches_pattern("/api/health", "/api/healthz"));
        assert!(!matches_pattern("/api/health", "/api"));
    }

    #[test]
    fn test_wildcard_match() {
        assert!(matches_pattern("/api/*", "/api/users"));
        assert!(matches_pattern("/api/*", "/api/users/123"));
        assert!(matches_pattern("/api/*", "/api/"));
        // Exact prefix without trailing slash
        assert!(matches_pattern("/api/*", "/api"));
        assert!(!matches_pattern("/api/*", "/other"));
    }

    #[test]
    fn test_root_wildcard() {
        assert!(matches_pattern("/*", "/anything"));
        assert!(matches_pattern("/*", "/a/b/c"));
    }

    #[test]
    fn test_segment_matching() {
        // Basic param capture
        let params = match_segments("/users/:id", "/users/42").unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].0, "id");
        assert_eq!(params[0].1, "42");

        // Multiple params
        let params = match_segments("/users/:user_id/posts/:post_id", "/users/7/posts/99").unwrap();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].0, "user_id");
        assert_eq!(params[0].1, "7");
        assert_eq!(params[1].0, "post_id");
        assert_eq!(params[1].1, "99");

        // Mixed literal and param
        let params = match_segments("/api/users/:id/profile", "/api/users/42/profile").unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].0, "id");
        assert_eq!(params[0].1, "42");
    }

    #[test]
    fn test_segment_no_match() {
        // Too many segments
        assert!(match_segments("/users/:id", "/users/42/extra").is_none());
        // Too few segments
        assert!(match_segments("/users/:id/posts", "/users/42").is_none());
        // Literal mismatch
        assert!(match_segments("/users/:id", "/posts/42").is_none());
    }

    #[test]
    fn test_exact_beats_param() {
        let router = SnowRouter {
            routes: vec![
                RouteEntry {
                    pattern: "/users/:id".to_string(),
                    method: None,
                    handler_fn: 1 as *mut u8,
                    handler_env: std::ptr::null_mut(),
                },
                RouteEntry {
                    pattern: "/users/me".to_string(),
                    method: None,
                    handler_fn: 2 as *mut u8,
                    handler_env: std::ptr::null_mut(),
                },
            ],
        };
        // Exact route "/users/me" should win over parameterized "/users/:id"
        // even though the parameterized route was registered first.
        let (fn_ptr, _, params) = router.match_route("/users/me", "GET").unwrap();
        assert_eq!(fn_ptr as usize, 2);
        assert!(params.is_empty());

        // Other paths should match the parameterized route.
        let (fn_ptr, _, params) = router.match_route("/users/42", "GET").unwrap();
        assert_eq!(fn_ptr as usize, 1);
        assert_eq!(params[0].0, "id");
        assert_eq!(params[0].1, "42");
    }

    #[test]
    fn test_method_filtering() {
        let router = SnowRouter {
            routes: vec![
                RouteEntry {
                    pattern: "/users".to_string(),
                    method: Some("GET".to_string()),
                    handler_fn: 1 as *mut u8,
                    handler_env: std::ptr::null_mut(),
                },
                RouteEntry {
                    pattern: "/users".to_string(),
                    method: Some("POST".to_string()),
                    handler_fn: 2 as *mut u8,
                    handler_env: std::ptr::null_mut(),
                },
            ],
        };
        // GET request should match the GET handler.
        let (fn_ptr, _, _) = router.match_route("/users", "GET").unwrap();
        assert_eq!(fn_ptr as usize, 1);

        // POST request should match the POST handler.
        let (fn_ptr, _, _) = router.match_route("/users", "POST").unwrap();
        assert_eq!(fn_ptr as usize, 2);

        // DELETE request should NOT match either.
        assert!(router.match_route("/users", "DELETE").is_none());
    }

    #[test]
    fn test_method_agnostic_route() {
        let router = SnowRouter {
            routes: vec![
                RouteEntry {
                    pattern: "/health".to_string(),
                    method: None,
                    handler_fn: 1 as *mut u8,
                    handler_env: std::ptr::null_mut(),
                },
            ],
        };
        // Method-agnostic route should match any method.
        assert!(router.match_route("/health", "GET").is_some());
        assert!(router.match_route("/health", "POST").is_some());
        assert!(router.match_route("/health", "DELETE").is_some());
    }

    #[test]
    fn test_router_match_order() {
        let router = SnowRouter {
            routes: vec![
                RouteEntry {
                    pattern: "/exact".to_string(),
                    method: None,
                    handler_fn: 1 as *mut u8,
                    handler_env: std::ptr::null_mut(),
                },
                RouteEntry {
                    pattern: "/*".to_string(),
                    method: None,
                    handler_fn: 2 as *mut u8,
                    handler_env: std::ptr::null_mut(),
                },
            ],
        };
        // Exact match should win (first in order).
        let (fn_ptr, _, _) = router.match_route("/exact", "GET").unwrap();
        assert_eq!(fn_ptr as usize, 1);

        // Wildcard catches the rest.
        let (fn_ptr, _, _) = router.match_route("/other", "GET").unwrap();
        assert_eq!(fn_ptr as usize, 2);
    }

    #[test]
    fn test_router_no_match() {
        let router = SnowRouter {
            routes: vec![RouteEntry {
                pattern: "/only-this".to_string(),
                method: None,
                handler_fn: 1 as *mut u8,
                handler_env: std::ptr::null_mut(),
            }],
        };
        assert!(router.match_route("/other", "GET").is_none());
    }

    #[test]
    fn test_snow_http_router_and_route() {
        snow_rt_init();

        let router = snow_http_router();
        assert!(!router.is_null());

        let pattern = snow_string_new(b"/hello".as_ptr(), 6);
        let handler_fn = 42usize as *mut u8;

        let router2 = snow_http_route(router, pattern, handler_fn);
        assert!(!router2.is_null());

        // Verify the new router has the route.
        unsafe {
            let r = &*(router2 as *const SnowRouter);
            assert_eq!(r.routes.len(), 1);
            assert_eq!(r.routes[0].pattern, "/hello");
            assert_eq!(r.routes[0].handler_fn as usize, 42);
            assert!(r.routes[0].method.is_none());
        }
    }
}
