# Technology Stack: Snow WebSocket Support

**Project:** Snow compiler -- WebSocket server (ws:// and wss://), RFC 6455 frame codec, actor-per-connection, rooms/channels, heartbeat
**Researched:** 2026-02-12
**Confidence:** HIGH (codebase analysis of snow-rt, verified crate availability via Cargo.toml and Cargo.lock, RFC 6455 specification)

## Recommended Stack

### NEW Dependency

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| sha1 | 0.10 | SHA-1 digest for WebSocket upgrade handshake (`Sec-WebSocket-Accept`) | The WebSocket handshake requires SHA-1(key + GUID). `sha1 0.10` is from the RustCrypto project (same family as `sha2 0.10` already used for PostgreSQL SCRAM). Pure Rust, no C dependencies, minimal API surface. Alternative: promote `ring 0.17` (already a transitive dep) and use `ring::digest::SHA1_FOR_LEGACY_USE_ONLY`. But `sha1` is explicit about purpose, smaller surface area, and adds ~200 lines to compilation vs ring's much larger API. |

### Existing Dependencies Reused (NO CHANGES)

| Technology | Version | New Use | Already Used For |
|------------|---------|---------|-----------------|
| base64 | 0.22 | Base64-encode SHA-1 hash for `Sec-WebSocket-Accept` header | PostgreSQL SCRAM-SHA-256 authentication |
| rustls | 0.23 | TLS for `wss://` via `WsStream::Tls` variant | PostgreSQL TLS (`PgStream`), HTTPS (`HttpStream`) |
| webpki-roots | 0.26 | Root CA certs for wss:// server cert (if needed) | PostgreSQL TLS client verification |
| rustls-pki-types | 1 | PEM cert/key loading for `Ws.serve_tls()` | HTTPS `Http.serve_tls()` |
| parking_lot | 0.12 | `Mutex<WsStream>` for reader/writer thread sharing, `RwLock` for room registry | Actor process locks, connection pool, process registry |
| rustc-hash | workspace | `FxHashMap`/`FxHashSet` for room subscriber sets | Process registry, scheduler internals |
| crossbeam-channel | 0.5 | (unchanged) Actor mailbox infrastructure | Actor message passing |
| corosensei | 0.3 | (unchanged) Coroutines for connection actors | M:N actor scheduler |

### Core Framework

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| Hand-rolled RFC 6455 frame codec | N/A | Parse and serialize WebSocket frames | Consistent with the project's approach: hand-rolled HTTP/1.1 parser in `server.rs`, hand-rolled PostgreSQL wire protocol in `pg.rs`. Frame format is simpler than either (2-14 byte header + payload). No external library needed. |
| Hand-rolled HTTP upgrade handshake | N/A | Validate Upgrade request, compute `Sec-WebSocket-Accept`, respond with HTTP 101 | Reuses the existing `parse_request()` function from `server.rs`. Only new code is the upgrade validation and 101 response writer. |

### Infrastructure

| Technology | Version | Purpose | Why |
|------------|---------|---------|-----|
| `std::net::TcpListener` | std | WebSocket server accept loop | Same as HTTP server. Blocking accept is appropriate for Snow's blocking I/O model. |
| `std::thread::spawn` | std | Reader thread per connection | Background OS thread for blocking frame reads. Same pattern as `snow_timer_send_after` which spawns OS threads. |
| `std::sync::atomic::AtomicBool` | std | Shutdown flag for reader thread | Reader thread checks this flag periodically to know when to exit. |

## Alternatives Considered

| Category | Recommended | Alternative | Why Not |
|----------|-------------|-------------|---------|
| WebSocket library | Hand-rolled | tungstenite | Pulls in `http`, `httparse`, `utf-8`, `thiserror`, `sha1` (6+ crates). Snow already has HTTP parsing and SHA-1 available. WebSocket frame parsing is ~200 lines. |
| WebSocket library | Hand-rolled | fastwebsockets | Depends on `hyper` and `tokio`. Incompatible with Snow's blocking I/O model. |
| WebSocket library | Hand-rolled | rust-websocket | Deprecated (uses Hyper 0.10, Tokio 0.1). |
| SHA-1 crate | sha1 0.10 | ring 0.17 (promote) | ring works but is a large crate with a large API surface. `sha1` is purpose-built, small, and from the same RustCrypto family as `sha2` already in use. |
| SHA-1 crate | sha1 0.10 | openssl | Would add a system dependency (libssl). Violates Snow's single-binary philosophy. |
| Room registry | `RwLock<HashMap>` | Room actor (message-passing) | A centralized room actor becomes a bottleneck for broadcast. Lock-based registry with direct `snow_actor_send()` fan-out is faster and simpler. |
| Room registry | `RwLock<HashMap>` | dashmap | Introduces a new concurrency primitive. `parking_lot::RwLock<HashMap>` is the existing pattern in the codebase (process registry). |
| Stream sharing | `Mutex<WsStream>` for TLS | Split TLS stream | `StreamOwned` cannot be split. `TcpStream::try_clone()` works for plain TCP but not for TLS. Mutex is the simplest correct approach. |
| Stream sharing | `TcpStream::try_clone()` for plain | Single-threaded with non-blocking reads | Would require non-blocking I/O + poll loop, changing the programming model. Too complex for the benefit. |

## Installation

```toml
# In crates/snow-rt/Cargo.toml, add ONE line:
# Phase XX: WebSocket upgrade handshake (SHA-1 for Sec-WebSocket-Accept)
# sha1 is from the RustCrypto project, same family as sha2 0.10 (already used).
sha1 = "0.10"
```

That is the entire dependency change for this milestone. Zero new transitive dependencies beyond `sha1` itself (which depends only on `digest` and `crypto-common`, both already compiled as deps of `sha2`).

## Version Pinning Summary

| Crate | Pin | Status | Source |
|-------|-----|--------|--------|
| sha1 | `"0.10"` | NEW direct dep | RustCrypto project, same family as existing sha2 |
| base64 | `"0.22"` | Existing | Cargo.toml line 26 |
| rustls | `"0.23"` | Existing | Cargo.toml line 30 |
| webpki-roots | `"0.26"` | Existing | Cargo.toml line 31 |
| parking_lot | `"0.12"` | Existing | Cargo.toml line 15 |
| rustc-hash | workspace | Existing | Cargo.toml line 16 |

## Sources

- [RFC 6455: The WebSocket Protocol](https://datatracker.ietf.org/doc/html/rfc6455) -- Handshake specification requiring SHA-1 + base64 (Section 4.2.2)
- [sha1 crate on crates.io](https://crates.io/crates/sha1) -- RustCrypto SHA-1 implementation
- [ring 0.17 digest docs](https://docs.rs/ring/0.17.14/ring/digest/index.html) -- Alternative SHA-1 via SHA1_FOR_LEGACY_USE_ONLY
- [base64 0.22 docs](https://docs.rs/base64/0.22.1/base64/index.html) -- BASE64_STANDARD.encode() API
- [tungstenite-rs GitHub](https://github.com/snapview/tungstenite-rs) -- Reference for frame codec patterns (not used as dependency)
- Snow codebase: `crates/snow-rt/Cargo.toml` -- current dependency list
- Snow codebase: `crates/snow-rt/src/http/server.rs` -- HttpStream enum, parse_request, accept loop
- Snow codebase: `crates/snow-rt/src/db/pg.rs` -- PgStream enum, hand-rolled wire protocol precedent
- Snow codebase: `crates/snow-rt/src/actor/mod.rs` -- snow_timer_send_after OS thread pattern

---
*Stack research for: Snow Language WebSocket Support*
*Researched: 2026-02-12*
