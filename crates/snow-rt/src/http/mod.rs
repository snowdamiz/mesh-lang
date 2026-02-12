//! HTTP module for the Snow standard library.
//!
//! Provides:
//! - **Router**: URL pattern matching with exact and wildcard routes
//! - **Server**: Blocking HTTP server with hand-rolled HTTP/1.1 parser, actor-per-connection
//! - **Client**: HTTP GET/POST requests using ureq
//! - **Request/Response**: Typed structs for request data and response construction
//!
//! ## Architecture
//!
//! The server uses the Snow actor system (corosensei coroutines on M:N
//! scheduler) for per-connection handling. Each incoming request is dispatched
//! to a lightweight actor with a 64 KiB stack, wrapped in `catch_unwind` for
//! crash isolation. Blocking I/O is accepted within the actor context (similar
//! to BEAM NIFs) since each actor runs on a scheduler worker thread.

pub mod client;
pub mod router;
pub mod server;

pub use client::{snow_http_get, snow_http_post};
pub use router::{
    snow_http_route, snow_http_route_delete, snow_http_route_get, snow_http_route_post,
    snow_http_route_put, snow_http_router, snow_http_use_middleware,
};
pub use server::{
    snow_http_request_body, snow_http_request_header, snow_http_request_method,
    snow_http_request_param, snow_http_request_path, snow_http_request_query,
    snow_http_response_new, snow_http_serve,
};
