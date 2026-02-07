//! HTTP module for the Snow standard library.
//!
//! Provides:
//! - **Router**: URL pattern matching with exact and wildcard routes
//! - **Server**: Blocking HTTP server using tiny_http with thread-per-connection
//! - **Client**: HTTP GET/POST requests using ureq
//! - **Request/Response**: Typed structs for request data and response construction
//!
//! ## Architecture
//!
//! The server uses `std::thread::spawn` for per-connection handling rather
//! than the actor runtime. This is a pragmatic choice: the actor runtime
//! uses corosensei coroutines with a work-stealing scheduler, and integrating
//! tiny-http's blocking I/O with cooperative scheduling introduces unnecessary
//! complexity. Thread-per-connection is simple and correct for HTTP serving.

pub mod client;
pub mod router;
pub mod server;

pub use client::{snow_http_get, snow_http_post};
pub use router::{snow_http_route, snow_http_router};
pub use server::{
    snow_http_request_body, snow_http_request_header, snow_http_request_method,
    snow_http_request_path, snow_http_request_query, snow_http_response_new, snow_http_serve,
};
