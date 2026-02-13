# Phase 64: Node Connection & Authentication - Research

**Researched:** 2026-02-12
**Domain:** TLS-encrypted inter-node TCP with cookie-based HMAC-SHA256 authentication and node discovery
**Confidence:** HIGH

## Summary

Phase 64 establishes the networking layer for Snow's distributed actor system. It must deliver five capabilities: (1) a node identity system where a runtime becomes a named, addressable entity via `Node.start("name@host", cookie: "secret")`; (2) a TCP listener that accepts incoming connections and a connector that initiates outgoing connections to remote nodes; (3) TLS encryption over those TCP connections using the existing rustls 0.23 infrastructure; (4) mutual authentication via HMAC-SHA256 cookie-based challenge/response during the post-TLS handshake; and (5) heartbeat-based dead connection detection.

The critical architectural decision is that this phase builds the **connection plumbing only** -- it does NOT route messages (Phase 65) or handle fault tolerance (Phase 66). The `dist_send_stub` remains a stub. Phase 64 delivers `NodeSession` objects that represent authenticated, encrypted connections between named nodes. Phase 65 will wire those sessions into the send path.

Snow's existing codebase provides all the building blocks: rustls 0.23 with ring crypto provider for TLS (used by HTTP/WS servers and PG client), HMAC-SHA256 via the `hmac` + `sha2` crates (used by PG SCRAM auth), `rand` for nonce generation, `parking_lot` for concurrent state, `rustc_hash::FxHashMap` for fast lookups, and the `ProcessRegistry` pattern for global name-to-value mappings. The zero-new-dependency constraint is fully satisfiable.

**Primary recommendation:** Build the node subsystem as a new `dist/node.rs` module (alongside the existing `dist/wire.rs`) containing: `NodeIdentity` (name + cookie + creation counter), `NodeRegistry` (global node table), `NodeSession` (per-connection state with TLS stream + heartbeat), and the handshake protocol. Use rustls `ServerConfig` with no client auth and `ClientConfig` with a custom `ServerCertVerifier` that skips CA validation (trust is established by the cookie challenge, not PKI). For inter-node TLS, generate an ephemeral ECDSA P-256 key pair and minimal self-signed DER certificate at node startup using ring's `EcdsaKeyPair::generate_pkcs8` -- the certificate need not be CA-trusted since verification is skipped.

## Standard Stack

### Core (already in Cargo.toml -- zero new dependencies)
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| rustls | 0.23.36 | TLS encryption for inter-node TCP streams | Already used for HTTP/WS servers and PG client |
| hmac | 0.12 | HMAC-SHA256 computation for cookie challenge/response | Already used in PG SCRAM-SHA-256 auth |
| sha2 | 0.10 | SHA-256 hashing for challenge digests | Already used in PG SCRAM auth |
| rand | 0.9 | Random nonce/challenge generation | Already used in PG SCRAM and WS Sec-WebSocket-Key |
| parking_lot | 0.12 | RwLock/Mutex for shared node state | Already used throughout actor system |
| rustc-hash | 2 | FxHashMap for node/session lookups | Already used in scheduler process table |
| ring | 0.17.14 | ECDSA key pair generation for ephemeral TLS certs (transitive dep) | Already compiled as rustls transitive dep |
| rustls-pki-types | 1.14 | Certificate and key DER types | Already a direct dependency |
| base64 | 0.22 | Encoding challenge/response payloads | Already used in PG SCRAM auth |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Custom HMAC-SHA256 challenge | Erlang-style MD5 challenge | MD5 is deprecated and weak; HMAC-SHA256 is already available and cryptographically strong |
| Ephemeral ECDSA cert | rcgen for proper X.509 | Would add a dependency; ephemeral self-signed is fine when we skip CA verification |
| Skip cert verification | Proper PKI with CA | Unnecessary complexity for inter-node auth; cookie challenge provides trust |
| Custom TLS handshake | Off-the-shelf mTLS | mTLS requires certificate distribution infrastructure; cookie is simpler for clusters |
| Built-in node registry | EPMD-like external daemon | External daemon adds operational complexity; built-in is simpler for Snow's use case |

**Installation:** No new dependencies. All work is within `snow-rt/src/dist/`.

## Architecture Patterns

### Recommended Project Structure
```
crates/snow-rt/src/
├── dist/
│   ├── mod.rs           # MODIFIED: re-export node types
│   ├── wire.rs          # EXISTING: STF encoder/decoder (Phase 63)
│   └── node.rs          # NEW: Node identity, registry, session, handshake, heartbeat
├── actor/
│   └── mod.rs           # MODIFIED: Add Node.start/connect extern "C" fns
└── lib.rs               # MODIFIED: Re-export node functions
```

### Pattern 1: Node Identity and Global State
**What:** A singleton `NodeState` holds the local node's identity (name, host, port, cookie, creation counter), the TLS server config, and a registry mapping node names to `NodeSession` objects.
**When to use:** Any code that needs to know "who am I?" or "who am I connected to?"
**Rationale:** Mirrors the `GLOBAL_SCHEDULER` / `GLOBAL_REGISTRY` pattern already used in the actor system.

```rust
// Source: Codebase pattern from actor/mod.rs:69 and actor/registry.rs:146
use std::sync::OnceLock;

pub struct NodeState {
    /// This node's identity: "name@host"
    pub name: String,
    /// Cookie for authentication
    pub cookie: String,
    /// Monotonically incrementing creation counter (wraps at 255)
    pub creation: AtomicU8,
    /// Assigned node_id for PID encoding (local is always 0, assigned to remote nodes)
    next_node_id: AtomicU16,
    /// TCP listener port
    pub port: u16,
    /// TLS server config (for accepting connections)
    pub tls_server_config: Arc<ServerConfig>,
    /// TLS client config (for initiating connections)
    pub tls_client_config: Arc<ClientConfig>,
    /// Connected nodes: name -> NodeSession
    pub sessions: RwLock<FxHashMap<String, Arc<NodeSession>>>,
    /// node_id -> node name (reverse mapping for PID routing in Phase 65)
    pub node_id_map: RwLock<FxHashMap<u16, String>>,
}

static NODE_STATE: OnceLock<NodeState> = OnceLock::new();
```

### Pattern 2: TLS Without PKI (Cookie Trust Model)
**What:** Use rustls for encryption but skip certificate chain verification. Trust is established by the cookie challenge/response that happens AFTER the TLS handshake completes.
**When to use:** All inter-node connections (both client and server side).
**Rationale:** Erlang/OTP does the same thing -- TLS provides confidentiality and integrity, while the cookie provides authentication. No need for a CA or certificate distribution.

```rust
// Source: rustls 0.23 docs + codebase pattern from db/pg.rs:328
// Server side: accept any client (no client auth required)
let server_config = ServerConfig::builder()
    .with_no_client_auth()
    .with_single_cert(vec![self_signed_cert], private_key)?;

// Client side: skip server certificate validation
// (cookie challenge provides authentication, not PKI)
#[derive(Debug)]
struct SkipCertVerification;
impl rustls::client::danger::ServerCertVerifier for SkipCertVerification {
    fn verify_server_cert(&self, ...) -> Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())
    }
    // ... delegate signature verification to ring
}

let client_config = ClientConfig::builder()
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(SkipCertVerification))
    .with_no_client_auth();
```

### Pattern 3: HMAC-SHA256 Cookie Challenge/Response
**What:** After TLS is established, both sides prove knowledge of the shared cookie via a challenge/response exchange. Each side sends a random challenge, and the other responds with `HMAC-SHA256(cookie, challenge)`.
**When to use:** During the handshake phase after TLS and name exchange.
**Rationale:** Stronger than Erlang's MD5-based challenge (which is deprecated). HMAC-SHA256 is already available and used in PG auth.

```rust
// Source: Codebase pattern from db/pg.rs:450-454
use hmac::{Hmac, Mac};
use sha2::Sha256;
type HmacSha256 = Hmac<Sha256>;

fn compute_challenge_response(cookie: &str, challenge: &[u8; 32]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(cookie.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(challenge);
    let result = mac.finalize().into_bytes();
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&result);
    digest
}
```

### Pattern 4: Handshake Protocol (Binary, Length-Prefixed)
**What:** A simple binary protocol for the post-TLS handshake. Each message is `[u32 length][u8 tag][payload]`. The handshake sequence is:
1. **Initiator sends `NAME`**: `[tag=1][u16 name_len][name_bytes][u8 creation]`
2. **Acceptor sends `STATUS+CHALLENGE`**: `[tag=2][u8 status][u32 challenge_len][challenge_bytes][u16 name_len][name_bytes][u8 creation]`
3. **Initiator sends `CHALLENGE_REPLY`**: `[tag=3][u32 digest_len][digest_bytes][u32 own_challenge_len][own_challenge_bytes]`
4. **Acceptor sends `CHALLENGE_ACK`**: `[tag=4][u32 digest_len][digest_bytes]`

**When to use:** During connection establishment, after TLS handshake completes.
**Rationale:** Mirrors Erlang's handshake flow but uses HMAC-SHA256 instead of MD5. Length-prefixed binary is consistent with STF wire format and PG wire protocol patterns.

### Pattern 5: Stream Abstraction (Blocking I/O on Dedicated Thread)
**What:** Each node connection uses a dedicated OS thread for the reader side (like WS server's reader thread pattern). The TLS stream is wrapped in `Arc<Mutex<TlsNodeStream>>` for shared read/write access.
**When to use:** For the reader loop that receives messages from a connected node.
**Rationale:** Snow's actor system uses blocking I/O with the M:N scheduler. The WS server already proves this pattern works for TLS streams. The reader thread pushes received messages into the appropriate actor's mailbox (in Phase 65).

```rust
// Source: Codebase pattern from ws/server.rs:51-54
pub(crate) enum NodeStream {
    Plain(TcpStream),  // For testing/development without TLS
    Tls(StreamOwned<ServerConnection, TcpStream>),
    TlsClient(StreamOwned<ClientConnection, TcpStream>),
}
```

### Pattern 6: Heartbeat via Dedicated Thread
**What:** Each `NodeSession` spawns a heartbeat thread that periodically writes a ping and expects a pong. If no pong arrives within the timeout, the connection is declared dead.
**When to use:** After a connection is fully authenticated and operational.
**Rationale:** Mirrors the WS server's `HeartbeatState` pattern (ws/server.rs:96-133). The default interval (60s) and timeout (15s) can be configured.

```rust
// Source: Codebase pattern from ws/server.rs:96-133
struct HeartbeatState {
    last_ping_sent: Instant,
    last_pong_received: Instant,
    ping_interval: Duration,    // default 60s (configurable, NODE-08)
    pong_timeout: Duration,     // default 15s
    pending_ping_payload: Option<[u8; 8]>,
}
```

### Anti-Patterns to Avoid
- **Using tokio for node connections:** Snow uses blocking I/O everywhere. Introducing async would be architecturally inconsistent and would fight the M:N scheduler.
- **Generating X.509 certificates manually:** DER encoding X.509 is complex and error-prone. Since we skip verification, the cert quality doesn't matter -- use a minimal DER stub or rcgen if available.
- **Storing the cookie in the process table:** The cookie is node-global state, not per-process. Use the `NodeState` singleton.
- **Exposing TcpStream directly:** Always wrap in a stream enum to support both plain (testing) and TLS (production) paths.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| TLS encryption | Custom crypto | rustls 0.23 `ServerConnection`/`ClientConnection` | TLS is insanely complex; rustls is audited and battle-tested |
| HMAC computation | Manual HMAC | `hmac` crate `Hmac<Sha256>` | HMAC requires careful constant-time comparison |
| Random challenges | `std::time` seeding | `rand::random::<[u8; 32]>()` | Cryptographic randomness is essential for security |
| Certificate DER types | Raw byte arrays | `rustls_pki_types::{CertificateDer, PrivateKeyDer}` | Proper type safety for the rustls API |
| Thread-safe state | Manual atomics | `parking_lot::RwLock<FxHashMap<..>>` | Matches existing codebase patterns |

**Key insight:** The entire crypto stack for this phase is already compiled into the binary via existing dependencies. This is purely a wiring and protocol design exercise, not a "choose your crypto library" exercise.

## Common Pitfalls

### Pitfall 1: Deadlock on Stream Mutex During Handshake
**What goes wrong:** The handshake reads and writes on the same TLS stream. If the reader thread starts before the handshake completes, it can hold the mutex while the handshake tries to write.
**Why it happens:** The WS server pattern spawns the reader thread after the handshake. If node connections try to share the stream before auth completes, deadlock ensues.
**How to avoid:** Complete the entire handshake (TLS + cookie challenge/response) on a single thread before splitting into reader thread + session object.
**Warning signs:** Connections that hang during establishment; test timeouts.

### Pitfall 2: TLS Certificate Generation at Startup
**What goes wrong:** Generating self-signed certificates requires ring's ECDSA key generation and minimal X.509 DER encoding. Getting the DER encoding wrong causes TLS to reject the certificate.
**Why it happens:** X.509 DER is notoriously complex. Even a minimal self-signed cert needs proper ASN.1 structure.
**How to avoid:** Two options: (A) Use a hardcoded minimal DER certificate template with only the key bytes substituted (safest), or (B) generate the minimal ASN.1 structure programmatically. Option A is strongly preferred -- the cert is never validated, so it just needs to be structurally valid enough for rustls to accept it as a `ServerConfig` input.
**Warning signs:** `rustls::Error` during `ServerConfig::builder().with_single_cert()`.

### Pitfall 3: Cookie Timing Attack
**What goes wrong:** Comparing HMAC digests with `==` leaks timing information about how many bytes match, potentially allowing an attacker to brute-force the cookie byte by byte.
**Why it happens:** Standard byte comparison short-circuits on first mismatch.
**How to avoid:** Use constant-time comparison. The `hmac` crate's `Mac::verify_slice` does this automatically. Alternatively, XOR all bytes and check if the result is zero.
**Warning signs:** None visible in testing; this is a security issue only exploitable in production.

### Pitfall 4: Simultaneous Connection Attempts
**What goes wrong:** Node A tries to connect to Node B at the same time Node B tries to connect to Node A. This creates two TCP connections for the same pair.
**Why it happens:** Both nodes discover each other and initiate connections concurrently.
**How to avoid:** Use a tiebreaker rule: the node with the lexicographically smaller name wins. If node A (smaller name) receives an incoming connection from B while A is already connecting to B, A's outgoing connection wins and B's is dropped. This is exactly how Erlang resolves it.
**Warning signs:** Duplicate connections in the session table; messages delivered twice.

### Pitfall 5: node_id Exhaustion and Reuse
**What goes wrong:** The 16-bit node_id space (65,535 values) can be exhausted if nodes connect and disconnect rapidly without reusing IDs.
**Why it happens:** Simple monotonic counter without recycling.
**How to avoid:** Reclaim node_id values when a node disconnects. Maintain a free list of previously used IDs. The creation counter (8-bit) distinguishes incarnations of the same node.
**Warning signs:** Node connections fail after 65K cumulative connections in a long-running cluster.

### Pitfall 6: Forgetting to Install ring Crypto Provider
**What goes wrong:** rustls panics with "no crypto provider installed" if ring isn't initialized.
**Why it happens:** rustls 0.23 requires explicit crypto provider installation.
**How to avoid:** `snow_rt_init()` in `gc.rs:106` already calls `rustls::crypto::ring::default_provider().install_default()`. Ensure `Node.start` is called after runtime init (which it will be, since it's a Snow function call).
**Warning signs:** Panic at connection time.

### Pitfall 7: Port Binding Failures
**What goes wrong:** `Node.start` binds a TCP port. If the port is already in use (another Snow node, or the OS hasn't released it), binding fails.
**Why it happens:** Default port assignment without checking availability.
**How to avoid:** Allow explicit port configuration OR use port 0 (OS-assigned) and report the actual port. Include the port in the node name format: `"name@host:port"` or `"name@host"` with a default/auto port.
**Warning signs:** "Address already in use" errors on startup.

## Code Examples

### Example 1: Node Startup (extern "C" API)
```rust
// Source: Architecture design based on codebase patterns
#[no_mangle]
pub extern "C" fn snow_node_start(
    name_ptr: *const u8,
    name_len: u64,
    cookie_ptr: *const u8,
    cookie_len: u64,
) -> i64 {
    let name = unsafe { std::str::from_utf8_unchecked(
        std::slice::from_raw_parts(name_ptr, name_len as usize)
    ) };
    let cookie = unsafe { std::str::from_utf8_unchecked(
        std::slice::from_raw_parts(cookie_ptr, cookie_len as usize)
    ) };

    // Parse "name@host" or "name@host:port"
    let (node_name, host, port) = parse_node_name(name);

    // Generate ephemeral TLS certificate
    let (cert_der, key_der) = generate_ephemeral_cert();

    // Build TLS configs
    let server_config = build_node_server_config(cert_der.clone(), key_der.clone());
    let client_config = build_node_client_config();

    // Initialize global NodeState
    NODE_STATE.get_or_init(|| NodeState::new(
        node_name, host, port, cookie, server_config, client_config,
    ));

    // Start TCP listener on a background thread
    start_listener(host, port);

    0 // success
}
```

### Example 2: HMAC-SHA256 Challenge/Response
```rust
// Source: Codebase pattern from db/pg.rs:450-454
use hmac::{Hmac, Mac};
use sha2::Sha256;
type HmacSha256 = Hmac<Sha256>;

/// Generate a 32-byte random challenge.
fn generate_challenge() -> [u8; 32] {
    rand::random()
}

/// Compute HMAC-SHA256(cookie, challenge) as the challenge response.
fn compute_response(cookie: &str, challenge: &[u8; 32]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(cookie.as_bytes())
        .expect("HMAC accepts any key size");
    mac.update(challenge);
    let result = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// Verify a challenge response using constant-time comparison.
fn verify_response(cookie: &str, challenge: &[u8; 32], response: &[u8; 32]) -> bool {
    let expected = compute_response(cookie, challenge);
    // Constant-time comparison via hmac crate
    let mut mac = HmacSha256::new_from_slice(cookie.as_bytes())
        .expect("HMAC accepts any key size");
    mac.update(challenge);
    mac.verify_slice(response).is_ok()
}
```

### Example 3: Connection Handshake Sequence
```rust
// Source: Architecture design inspired by Erlang dist protocol + existing WS handshake
fn perform_node_handshake(
    stream: &mut impl Read + Write,
    local_state: &NodeState,
    is_initiator: bool,
) -> Result<(String, u8, u16), String> {
    if is_initiator {
        // Step 1: Send our name
        send_name(stream, &local_state.name, local_state.creation())?;

        // Step 2: Receive their name + challenge
        let (remote_name, remote_creation, their_challenge) = recv_challenge(stream)?;

        // Step 3: Compute response to their challenge + send our challenge
        let our_response = compute_response(&local_state.cookie, &their_challenge);
        let our_challenge = generate_challenge();
        send_challenge_reply(stream, &our_response, &our_challenge)?;

        // Step 4: Receive their response to our challenge
        let their_response = recv_challenge_ack(stream)?;
        if !verify_response(&local_state.cookie, &our_challenge, &their_response) {
            return Err("cookie mismatch: authentication failed".to_string());
        }

        Ok((remote_name, remote_creation, assign_node_id()))
    } else {
        // Acceptor flow (mirror of above)
        // ...
    }
}
```

### Example 4: Heartbeat Thread
```rust
// Source: Codebase pattern from ws/server.rs:639-778
fn heartbeat_loop(
    stream: Arc<Mutex<NodeStream>>,
    shutdown: Arc<AtomicBool>,
    interval: Duration,    // default 60s
    timeout: Duration,     // default 15s
) {
    let mut state = HeartbeatState::new(interval, timeout);
    loop {
        if shutdown.load(Ordering::SeqCst) { break; }

        if state.is_pong_overdue() {
            // Dead connection detected
            shutdown.store(true, Ordering::SeqCst);
            // Phase 66 will fire :nodedown signals here
            break;
        }

        if state.should_send_ping() {
            let payload: [u8; 8] = rand::random();
            let mut s = stream.lock();
            let _ = write_heartbeat_ping(&mut *s, &payload);
            state.last_ping_sent = Instant::now();
            state.pending_ping_payload = Some(payload);
        }

        // Brief sleep to avoid busy-wait
        std::thread::sleep(Duration::from_millis(100));
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Erlang MD5 cookie challenge | HMAC-SHA256 cookie challenge | Snow design decision | Cryptographically stronger; uses existing deps |
| EPMD external daemon | Built-in node registry | Snow design decision | Simpler deployment; no external process needed |
| Separate TLS + auth layers | TLS encryption + cookie auth over same stream | Snow design decision | Single connection, simpler protocol |
| plaintext distribution + optional TLS | TLS-by-default (NODE-03) | Snow design decision | Security by default |

**Deprecated/outdated:**
- Erlang's MD5-based challenge (deprecated since OTP 23; Erlang recommends TLS for distribution)
- EPMD as mandatory infrastructure (Erlang 21+ supports `-epmd_module` for custom discovery)

## Open Questions

1. **Ephemeral TLS Certificate Generation**
   - What we know: ring 0.17 supports `EcdsaKeyPair::generate_pkcs8()` for key generation. We need a DER-encoded self-signed X.509 certificate to feed to `ServerConfig::with_single_cert()`.
   - What's unclear: Can we construct a minimal valid DER certificate without adding rcgen? The X.509 DER format is complex, but a minimal self-signed cert (without extensions, with a static issuer/subject) is achievable with ~200 bytes of hand-crafted ASN.1.
   - Recommendation: Start with a hardcoded DER template approach -- a byte array with placeholders for the public key and signature. If this proves too fragile, fall back to loading a user-provided cert/key pair (like the HTTP/WS TLS servers do), with auto-generation as a stretch goal.

2. **Port Assignment Strategy**
   - What we know: The success criteria says `Node.start("name@host", cookie: "secret")`. There's no explicit port in the API.
   - What's unclear: Should port be part of the name (like `"name@host:9001"`) or auto-assigned? How does `Node.connect("name@host:port")` know the port if it's not in the name?
   - Recommendation: Use format `"name@host"` for `Node.start` with a default port (e.g., 9000) or configurable via optional parameter. Use `"name@host:port"` for `Node.connect` where the port is always explicit. This mirrors Erlang's `name@host` for start + explicit port for connect.

3. **Simultaneous Connection Resolution**
   - What we know: Erlang uses the "alive" handshake to detect and resolve simultaneous connections.
   - What's unclear: Is this needed for Phase 64 (which has no mesh formation) or can it be deferred to Phase 65?
   - Recommendation: Implement the tiebreaker in Phase 64. It's simpler to build it into the handshake than to retrofit it later. Use lexicographic name comparison as the tiebreaker.

## Sources

### Primary (HIGH confidence)
- **Codebase analysis** (direct file reads):
  - `crates/snow-rt/src/dist/mod.rs` and `dist/wire.rs` -- existing STF module structure
  - `crates/snow-rt/src/actor/process.rs` -- PID bit-packing with node_id/creation/local_id
  - `crates/snow-rt/src/actor/mod.rs` -- dist_send_stub, local_send, extern "C" API pattern
  - `crates/snow-rt/src/actor/registry.rs` -- ProcessRegistry pattern (global OnceLock + RwLock)
  - `crates/snow-rt/src/actor/scheduler.rs` -- ProcessTable type, Scheduler struct
  - `crates/snow-rt/src/db/pg.rs` -- HMAC-SHA256/SCRAM auth, TLS client config, stream abstraction
  - `crates/snow-rt/src/http/server.rs` -- TLS ServerConfig building, stream enum pattern
  - `crates/snow-rt/src/ws/server.rs` -- HeartbeatState, reader thread pattern, WsStream enum
  - `crates/snow-rt/src/gc.rs:106` -- ring crypto provider installation
  - `crates/snow-rt/Cargo.toml` -- all dependencies confirmed present
  - `.planning/ROADMAP.md` -- Phase 64-69 requirements and success criteria

### Secondary (MEDIUM confidence)
- [rustls 0.23 ServerCertVerifier danger module](https://docs.rs/rustls/latest/rustls/client/danger/trait.ServerCertVerifier.html) -- Custom cert verifier API for skipping validation
- [ring Ed25519KeyPair and EcdsaKeyPair](https://docs.rs/ring/latest/ring/signature/struct.EcdsaKeyPair.html) -- Key generation support in ring 0.17
- [Erlang Distribution Protocol](https://www.erlang.org/doc/apps/erts/erl_dist_protocol.html) -- Challenge/response handshake design reference
- [EEF Security WG: Distribution Protocol](https://erlef.github.io/security-wg/secure_coding_and_deployment_hardening/distribution.html) -- Security analysis of Erlang's approach

### Tertiary (LOW confidence)
- [Quinn Certificate Configuration](https://quinn-rs.github.io/quinn/quinn/certificate.html) -- Example of SkipServerVerification pattern (verified against rustls 0.23 docs)

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all dependencies already in Cargo.toml; zero additions needed
- Architecture: HIGH -- patterns directly mirror existing HTTP/WS/PG code in the same crate
- Pitfalls: HIGH -- derived from analyzing actual code paths and known distributed systems issues
- TLS cert generation: MEDIUM -- ring supports key gen but minimal DER construction needs validation
- Protocol design: HIGH -- based on well-understood Erlang distribution protocol with modernized crypto

**Research date:** 2026-02-12
**Valid until:** 2026-03-14 (stable domain; rustls/ring APIs are stable)
