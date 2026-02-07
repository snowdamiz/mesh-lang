//! HTTP router for the Snow runtime.
//!
//! Routes are checked in registration order; first match wins.
//! Supports exact match and wildcard patterns (`/api/*` matches any
//! path starting with `/api/`).

use crate::gc::snow_gc_alloc;
use crate::string::SnowString;

/// A single route entry mapping a URL pattern to a handler.
pub struct RouteEntry {
    /// URL pattern (exact or wildcard ending with `/*`).
    pub pattern: String,
    /// Pointer to the handler function.
    pub handler_fn: *mut u8,
    /// Pointer to the handler closure environment (null for bare functions).
    pub handler_env: *mut u8,
}

/// Router holding an ordered list of route entries.
pub struct SnowRouter {
    pub routes: Vec<RouteEntry>,
}

impl SnowRouter {
    /// Find the first route matching the given path.
    /// Returns (handler_fn, handler_env) or None.
    pub fn match_route(&self, path: &str) -> Option<(*mut u8, *mut u8)> {
        for entry in &self.routes {
            if matches_pattern(&entry.pattern, path) {
                return Some((entry.handler_fn, entry.handler_env));
            }
        }
        None
    }
}

/// Check if a pattern matches a path.
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

// ── Public extern "C" API ──────────────────────────────────────────────

/// Create an empty router. Returns a pointer to a heap-allocated SnowRouter.
#[no_mangle]
pub extern "C" fn snow_http_router() -> *mut u8 {
    let router = Box::new(SnowRouter {
        routes: Vec::new(),
    });
    Box::into_raw(router) as *mut u8
}

/// Add a route to the router. Returns a NEW router pointer (immutable semantics).
///
/// # Arguments
/// - `router`: pointer to an existing SnowRouter
/// - `pattern`: pointer to a SnowString URL pattern
/// - `handler_fn`: function pointer for the route handler
/// - `handler_env`: closure environment pointer (null for bare functions)
#[no_mangle]
pub extern "C" fn snow_http_route(
    router: *mut u8,
    pattern: *const SnowString,
    handler_fn: *mut u8,
    handler_env: *mut u8,
) -> *mut u8 {
    unsafe {
        let old = &*(router as *const SnowRouter);
        let pat_str = (*pattern).as_str().to_string();

        // Clone existing routes and add new one.
        let mut new_routes = Vec::with_capacity(old.routes.len() + 1);
        for entry in &old.routes {
            new_routes.push(RouteEntry {
                pattern: entry.pattern.clone(),
                handler_fn: entry.handler_fn,
                handler_env: entry.handler_env,
            });
        }
        new_routes.push(RouteEntry {
            pattern: pat_str,
            handler_fn,
            handler_env,
        });

        let new_router = Box::new(SnowRouter {
            routes: new_routes,
        });
        Box::into_raw(new_router) as *mut u8
    }
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
    fn test_router_match_order() {
        let router = SnowRouter {
            routes: vec![
                RouteEntry {
                    pattern: "/exact".to_string(),
                    handler_fn: 1 as *mut u8,
                    handler_env: std::ptr::null_mut(),
                },
                RouteEntry {
                    pattern: "/*".to_string(),
                    handler_fn: 2 as *mut u8,
                    handler_env: std::ptr::null_mut(),
                },
            ],
        };
        // Exact match should win (first in order).
        let (fn_ptr, _) = router.match_route("/exact").unwrap();
        assert_eq!(fn_ptr as usize, 1);

        // Wildcard catches the rest.
        let (fn_ptr, _) = router.match_route("/other").unwrap();
        assert_eq!(fn_ptr as usize, 2);
    }

    #[test]
    fn test_router_no_match() {
        let router = SnowRouter {
            routes: vec![RouteEntry {
                pattern: "/only-this".to_string(),
                handler_fn: 1 as *mut u8,
                handler_env: std::ptr::null_mut(),
            }],
        };
        assert!(router.match_route("/other").is_none());
    }

    #[test]
    fn test_snow_http_router_and_route() {
        snow_rt_init();

        let router = snow_http_router();
        assert!(!router.is_null());

        let pattern = snow_string_new(b"/hello".as_ptr(), 6);
        let handler_fn = 42usize as *mut u8;
        let handler_env = std::ptr::null_mut();

        let router2 = snow_http_route(router, pattern, handler_fn, handler_env);
        assert!(!router2.is_null());

        // Verify the new router has the route.
        unsafe {
            let r = &*(router2 as *const SnowRouter);
            assert_eq!(r.routes.len(), 1);
            assert_eq!(r.routes[0].pattern, "/hello");
            assert_eq!(r.routes[0].handler_fn as usize, 42);
        }
    }
}
