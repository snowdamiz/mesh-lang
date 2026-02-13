//! HTTP module for the Mesh standard library.
//!
//! Provides:
//! - **Router**: URL pattern matching with exact and wildcard routes
//! - **Server**: Blocking HTTP/HTTPS server with hand-rolled HTTP/1.1 parser, actor-per-connection
//! - **Client**: HTTP GET/POST requests using ureq
//! - **Request/Response**: Typed structs for request data and response construction
//!
//! ## Architecture
//!
//! The server uses the Mesh actor system (corosensei coroutines on M:N
//! scheduler) for per-connection handling. Each incoming request is dispatched
//! to a lightweight actor with a 64 KiB stack, wrapped in `catch_unwind` for
//! crash isolation. Blocking I/O is accepted within the actor context (similar
//! to BEAM NIFs) since each actor runs on a scheduler worker thread.
//!
//! Both plaintext HTTP (`mesh_http_serve`) and HTTPS (`mesh_http_serve_tls`)
//! share the same actor infrastructure via the `HttpStream` enum, which
//! dispatches between `TcpStream` and `StreamOwned<ServerConnection, TcpStream>`.

pub mod client;
pub mod router;
pub mod server;

pub use client::{mesh_http_get, mesh_http_post};
pub use router::{
    mesh_http_route, mesh_http_route_delete, mesh_http_route_get, mesh_http_route_post,
    mesh_http_route_put, mesh_http_router, mesh_http_use_middleware,
};
pub use server::{
    mesh_http_request_body, mesh_http_request_header, mesh_http_request_method,
    mesh_http_request_param, mesh_http_request_path, mesh_http_request_query,
    mesh_http_response_new, mesh_http_serve, mesh_http_serve_tls,
};
