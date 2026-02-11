//! HTTP server runtime for the Snow language.
//!
//! Uses `tiny_http` for the HTTP server and the Snow actor system for
//! per-connection handling. Each incoming connection is dispatched to a
//! lightweight actor (corosensei coroutine on the M:N scheduler) rather
//! than an OS thread, benefiting from 64 KiB stacks and crash isolation
//! via `catch_unwind`.
//!
//! ## History
//!
//! Phase 8 used `std::thread::spawn` for per-connection handling. Phase 15
//! replaced this with actor-per-connection using the existing lightweight
//! actor system, unifying the runtime model. Blocking I/O in tiny-http is
//! accepted (similar to BEAM NIFs) since each actor runs on a scheduler
//! worker thread.

use crate::actor;
use crate::collections::map;
use crate::gc::snow_gc_alloc_actor;
use crate::string::{snow_string_new, SnowString};

use super::router::SnowRouter;

// ── Request/Response structs ────────────────────────────────────────────

/// HTTP request representation passed to Snow handler functions.
///
/// All fields are opaque pointers at the LLVM level. The Snow program
/// accesses them via accessor functions (request_method, request_path, etc.).
///
/// IMPORTANT: This struct is `#[repr(C)]` -- new fields MUST be appended
/// at the end to preserve existing field offsets.
#[repr(C)]
pub struct SnowHttpRequest {
    /// HTTP method as SnowString (e.g. "GET", "POST").
    pub method: *mut u8,
    /// Request path as SnowString (e.g. "/api/users").
    pub path: *mut u8,
    /// Request body as SnowString (empty string for GET).
    pub body: *mut u8,
    /// Query parameters as SnowMap (string keys -> string values).
    pub query_params: *mut u8,
    /// Headers as SnowMap (string keys -> string values).
    pub headers: *mut u8,
    /// Path parameters as SnowMap (string keys -> string values).
    /// Populated by the router when matching parameterized routes.
    pub path_params: *mut u8,
}

/// HTTP response returned by Snow handler functions.
#[repr(C)]
pub struct SnowHttpResponse {
    /// HTTP status code (e.g. 200, 404).
    pub status: i64,
    /// Response body as SnowString.
    pub body: *mut u8,
}

// ── Response constructor ───────────────────────────────────────────────

/// Create a new HTTP response with the given status code and body.
#[no_mangle]
pub extern "C" fn snow_http_response_new(status: i64, body: *const SnowString) -> *mut u8 {
    unsafe {
        let ptr = snow_gc_alloc_actor(
            std::mem::size_of::<SnowHttpResponse>() as u64,
            std::mem::align_of::<SnowHttpResponse>() as u64,
        ) as *mut SnowHttpResponse;
        (*ptr).status = status;
        (*ptr).body = body as *mut u8;
        ptr as *mut u8
    }
}

// ── Request accessors ──────────────────────────────────────────────────

/// Get the HTTP method from a request.
#[no_mangle]
pub extern "C" fn snow_http_request_method(req: *mut u8) -> *mut u8 {
    unsafe { (*(req as *const SnowHttpRequest)).method }
}

/// Get the URL path from a request.
#[no_mangle]
pub extern "C" fn snow_http_request_path(req: *mut u8) -> *mut u8 {
    unsafe { (*(req as *const SnowHttpRequest)).path }
}

/// Get the request body.
#[no_mangle]
pub extern "C" fn snow_http_request_body(req: *mut u8) -> *mut u8 {
    unsafe { (*(req as *const SnowHttpRequest)).body }
}

/// Get the value of a request header by name. Returns SnowOption
/// (tag 0 = Some with SnowString, tag 1 = None).
#[no_mangle]
pub extern "C" fn snow_http_request_header(req: *mut u8, name: *const SnowString) -> *mut u8 {
    unsafe {
        let request = &*(req as *const SnowHttpRequest);
        let key_str = (*name).as_str();
        // Look up in the headers map. Keys are SnowString pointers stored as u64.
        let key_snow = snow_string_new(key_str.as_ptr(), key_str.len() as u64);
        let val = map::snow_map_get(request.headers, key_snow as u64);
        if val == 0 {
            // None
            alloc_option(1, std::ptr::null_mut())
        } else {
            // Some -- val is the SnowString pointer stored as u64
            alloc_option(0, val as *mut u8)
        }
    }
}

/// Get the value of a query parameter by name. Returns SnowOption
/// (tag 0 = Some with SnowString, tag 1 = None).
#[no_mangle]
pub extern "C" fn snow_http_request_query(req: *mut u8, name: *const SnowString) -> *mut u8 {
    unsafe {
        let request = &*(req as *const SnowHttpRequest);
        let key_str = (*name).as_str();
        let key_snow = snow_string_new(key_str.as_ptr(), key_str.len() as u64);
        let val = map::snow_map_get(request.query_params, key_snow as u64);
        if val == 0 {
            alloc_option(1, std::ptr::null_mut())
        } else {
            alloc_option(0, val as *mut u8)
        }
    }
}

/// Get the value of a path parameter by name. Returns SnowOption
/// (tag 0 = Some with SnowString, tag 1 = None).
///
/// Path parameters are extracted from parameterized route patterns
/// like `/users/:id`. For a request matching this pattern with path
/// `/users/42`, `Request.param(req, "id")` returns `Some("42")`.
#[no_mangle]
pub extern "C" fn snow_http_request_param(req: *mut u8, name: *const SnowString) -> *mut u8 {
    unsafe {
        let request = &*(req as *const SnowHttpRequest);
        let key_str = (*name).as_str();
        let key_snow = snow_string_new(key_str.as_ptr(), key_str.len() as u64);
        let val = map::snow_map_get(request.path_params, key_snow as u64);
        if val == 0 {
            alloc_option(1, std::ptr::null_mut())
        } else {
            alloc_option(0, val as *mut u8)
        }
    }
}

// ── Option allocation helper (shared from crate::option) ────────────────

fn alloc_option(tag: u8, value: *mut u8) -> *mut u8 {
    crate::option::alloc_option(tag, value) as *mut u8
}

// ── Actor-per-connection infrastructure ────────────────────────────────

/// Arguments passed to the connection handler actor via raw pointer.
#[repr(C)]
struct ConnectionArgs {
    /// Router address as usize (for Send safety across thread boundaries).
    router_addr: usize,
    /// Raw pointer to a boxed tiny_http::Request, transferred as usize.
    request_ptr: usize,
}

/// Actor entry function for handling a single HTTP connection.
///
/// Receives a raw pointer to `ConnectionArgs` containing the router
/// address and a boxed `tiny_http::Request`. Wraps the handler call
/// in `catch_unwind` for crash isolation -- a panic in one handler
/// does not affect other connections.
extern "C" fn connection_handler_entry(args: *const u8) {
    if args.is_null() {
        return;
    }

    let args = unsafe { Box::from_raw(args as *mut ConnectionArgs) };
    let router_ptr = args.router_addr as *mut u8;
    let request = unsafe { *Box::from_raw(args.request_ptr as *mut tiny_http::Request) };

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        handle_request(router_ptr, request);
    }));

    if let Err(panic_info) = result {
        eprintln!("[snow-rt] HTTP handler panicked: {:?}", panic_info);
    }
}

// ── Server ─────────────────────────────────────────────────────────────

/// Start an HTTP server on the given port, blocking the calling thread.
///
/// The server listens for incoming connections and dispatches each
/// request to a lightweight actor via the Snow actor scheduler. Each
/// connection handler runs as a coroutine (64 KiB stack) with crash
/// isolation via `catch_unwind` in `connection_handler_entry`.
///
/// Handler calling convention (same as closures in collections):
/// - If handler_env is null: `fn(request_ptr) -> response_ptr`
/// - If handler_env is non-null: `fn(handler_env, request_ptr) -> response_ptr`
#[no_mangle]
pub extern "C" fn snow_http_serve(router: *mut u8, port: i64) {
    // Ensure the actor scheduler is initialized (idempotent).
    crate::actor::snow_rt_init_actor(0);

    let addr = format!("0.0.0.0:{}", port);
    let server = match tiny_http::Server::http(&addr) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[snow-rt] Failed to start HTTP server on {}: {}", addr, e);
            return;
        }
    };

    eprintln!("[snow-rt] HTTP server listening on {}", addr);

    let router_addr = router as usize;

    for request in server.incoming_requests() {
        let request_ptr = Box::into_raw(Box::new(request)) as usize;
        let args = ConnectionArgs {
            router_addr,
            request_ptr,
        };
        let args_ptr = Box::into_raw(Box::new(args)) as *const u8;
        let args_size = std::mem::size_of::<ConnectionArgs>() as u64;

        let sched = actor::global_scheduler();
        sched.spawn(
            connection_handler_entry as *const u8,
            args_ptr,
            args_size,
            1, // Normal priority
        );
    }
}

/// Handle a single HTTP request by matching it against the router
/// and calling the appropriate handler function.
fn handle_request(router_ptr: *mut u8, mut request: tiny_http::Request) {
    unsafe {
        let router = &*(router_ptr as *const SnowRouter);

        // Build the SnowHttpRequest.
        let method_str = request.method().as_str().to_string();
        let method = snow_string_new(method_str.as_ptr(), method_str.len() as u64) as *mut u8;

        let url = request.url().to_string();
        // Split URL into path and query string.
        let (path_str, query_str) = match url.find('?') {
            Some(idx) => (&url[..idx], &url[idx + 1..]),
            None => (url.as_str(), ""),
        };
        let path = snow_string_new(path_str.as_ptr(), path_str.len() as u64) as *mut u8;

        // Read body.
        let mut body_bytes = Vec::new();
        let _ = request.as_reader().read_to_end(&mut body_bytes);
        let body = snow_string_new(body_bytes.as_ptr(), body_bytes.len() as u64) as *mut u8;

        // Parse query params into a SnowMap.
        let mut query_map = map::snow_map_new();
        if !query_str.is_empty() {
            for param in query_str.split('&') {
                if let Some((k, v)) = param.split_once('=') {
                    let key = snow_string_new(k.as_ptr(), k.len() as u64);
                    let val = snow_string_new(v.as_ptr(), v.len() as u64);
                    query_map = map::snow_map_put(query_map, key as u64, val as u64);
                }
            }
        }

        // Parse headers into a SnowMap.
        let mut headers_map = map::snow_map_new();
        for header in request.headers() {
            let name = header.field.as_str().as_str();
            let value_str = header.value.as_str();
            let key = snow_string_new(name.as_ptr(), name.len() as u64);
            let val = snow_string_new(value_str.as_ptr(), value_str.len() as u64);
            headers_map = map::snow_map_put(headers_map, key as u64, val as u64);
        }

        // Match against router (now with method and path params).
        if let Some((handler_fn, handler_env, params)) = router.match_route(path_str, &method_str) {
            // Convert captured path params into a SnowMap.
            let mut path_params_map = map::snow_map_new();
            for (k, v) in &params {
                let key = snow_string_new(k.as_ptr(), k.len() as u64);
                let val = snow_string_new(v.as_ptr(), v.len() as u64);
                path_params_map = map::snow_map_put(path_params_map, key as u64, val as u64);
            }

            // Build the request struct.
            let snow_req = snow_gc_alloc_actor(
                std::mem::size_of::<SnowHttpRequest>() as u64,
                std::mem::align_of::<SnowHttpRequest>() as u64,
            ) as *mut SnowHttpRequest;
            (*snow_req).method = method;
            (*snow_req).path = path;
            (*snow_req).body = body;
            (*snow_req).query_params = query_map;
            (*snow_req).headers = headers_map;
            (*snow_req).path_params = path_params_map;

            let req_ptr = snow_req as *mut u8;

            // Call handler.
            let response_ptr = if handler_env.is_null() {
                let f: fn(*mut u8) -> *mut u8 = std::mem::transmute(handler_fn);
                f(req_ptr)
            } else {
                let f: fn(*mut u8, *mut u8) -> *mut u8 = std::mem::transmute(handler_fn);
                f(handler_env, req_ptr)
            };

            // Extract response.
            let resp = &*(response_ptr as *const SnowHttpResponse);
            let status_code = resp.status as u32;
            let body_str = if resp.body.is_null() {
                ""
            } else {
                let body_snow = &*(resp.body as *const SnowString);
                body_snow.as_str()
            };

            let http_response = tiny_http::Response::from_string(body_str)
                .with_status_code(tiny_http::StatusCode(status_code as u16))
                .with_header(
                    tiny_http::Header::from_bytes(
                        &b"Content-Type"[..],
                        &b"application/json; charset=utf-8"[..],
                    )
                    .unwrap(),
                );
            let _ = request.respond(http_response);
        } else {
            // 404 Not Found.
            let not_found = tiny_http::Response::from_string("Not Found")
                .with_status_code(tiny_http::StatusCode(404));
            let _ = request.respond(not_found);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::snow_rt_init;

    #[test]
    fn test_response_creation() {
        snow_rt_init();
        let body = snow_string_new(b"Hello".as_ptr(), 5);
        let resp_ptr = snow_http_response_new(200, body);
        assert!(!resp_ptr.is_null());
        unsafe {
            let resp = &*(resp_ptr as *const SnowHttpResponse);
            assert_eq!(resp.status, 200);
            let body_str = &*(resp.body as *const SnowString);
            assert_eq!(body_str.as_str(), "Hello");
        }
    }

    #[test]
    fn test_request_accessors() {
        snow_rt_init();

        // Build a request manually.
        let method = snow_string_new(b"GET".as_ptr(), 3) as *mut u8;
        let path = snow_string_new(b"/test".as_ptr(), 5) as *mut u8;
        let body = snow_string_new(b"".as_ptr(), 0) as *mut u8;
        let query_params = map::snow_map_new();
        let headers = map::snow_map_new();
        let path_params = map::snow_map_new();

        unsafe {
            let req_ptr = snow_gc_alloc_actor(
                std::mem::size_of::<SnowHttpRequest>() as u64,
                std::mem::align_of::<SnowHttpRequest>() as u64,
            ) as *mut SnowHttpRequest;
            (*req_ptr).method = method;
            (*req_ptr).path = path;
            (*req_ptr).body = body;
            (*req_ptr).query_params = query_params;
            (*req_ptr).headers = headers;
            (*req_ptr).path_params = path_params;

            let req = req_ptr as *mut u8;

            // Test method accessor.
            let m = snow_http_request_method(req);
            let m_str = &*(m as *const SnowString);
            assert_eq!(m_str.as_str(), "GET");

            // Test path accessor.
            let p = snow_http_request_path(req);
            let p_str = &*(p as *const SnowString);
            assert_eq!(p_str.as_str(), "/test");

            // Test body accessor.
            let b = snow_http_request_body(req);
            let b_str = &*(b as *const SnowString);
            assert_eq!(b_str.as_str(), "");
        }
    }
}
