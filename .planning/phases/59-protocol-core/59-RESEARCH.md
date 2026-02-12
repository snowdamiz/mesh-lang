# Phase 59: Protocol Core - Research

**Researched:** 2026-02-12
**Domain:** RFC 6455 WebSocket protocol -- frame codec, HTTP upgrade handshake, masking, close handshake
**Confidence:** HIGH

## Summary

Phase 59 implements the WebSocket wire protocol layer in `snow-rt`. This is a pure Rust runtime phase with no compiler changes -- it produces functions that Phase 60 will wire into the codegen as `Ws.*` builtins. The scope covers: (1) parsing the HTTP upgrade request and computing the `Sec-WebSocket-Accept` header per RFC 6455, (2) a frame parser that handles the variable-length header format (2-14 bytes), three payload length encodings, FIN bit, and opcode dispatch, (3) XOR-based unmasking of client-to-server frames, (4) writing unmasked server-to-client frames, (5) UTF-8 validation for text frames, (6) raw byte delivery for binary frames, (7) two-phase close handshake with status codes, and (8) error handling for malformed upgrades (HTTP 400) and unknown opcodes (close code 1002).

The existing codebase provides a strong foundation. The HTTP request parser in `server.rs` already handles `GET` requests with header extraction, which is exactly what the upgrade handshake needs. The `base64` crate (0.22) and `sha2` crate (0.10, same RustCrypto family) are already dependencies; adding `sha1 = "0.10"` uses the identical `Digest` trait API. The `HttpStream` enum (Plain/Tls) provides the stream abstraction that WebSocket connections will read/write through. No new paradigms are needed -- this phase extends the existing HTTP infrastructure with WebSocket-specific protocol handling.

The critical design decision is where the WebSocket code lives. It should go in a new `crates/snow-rt/src/ws/` module directory, parallel to `http/`, with `mod.rs`, `handshake.rs`, `frame.rs`, and `close.rs`. The handshake module reuses `parse_request` from the HTTP server to read the initial GET request, then performs the WebSocket-specific validation and response. The frame module is self-contained: a `WsFrame` struct with `parse` and `write` methods operating on `Read + Write` streams. This separation keeps Phase 59's frame codec cleanly testable in isolation, and Phase 60 can import it without entangling HTTP router logic.

**Primary recommendation:** Build the WebSocket protocol layer as a new `ws/` module in snow-rt, organized as handshake + frame codec + close logic. Add `sha1 = "0.10"` as the only new dependency. Unit-test everything with in-memory byte buffers (no network needed for protocol-level tests). The handshake reuses the existing HTTP parser; the frame codec is a new, self-contained state machine.

## Standard Stack

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `sha1` | 0.10 | SHA-1 hash for Sec-WebSocket-Accept | RFC 6455 mandates SHA-1. Same RustCrypto Digest trait as existing sha2 0.10. |
| `base64` | 0.22 | Base64 encode/decode for handshake | Already a dependency. Used in PG SCRAM auth (Phase 54). |
| `std::io::{Read, Write, BufReader}` | stdlib | Stream I/O for frame reading/writing | Same pattern as HTTP parser. |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| `sha2` | 0.10 | (existing, unchanged) | Not used in this phase, but sha1 shares the same digest crate. |
| `rustls` | 0.23 | (existing, unchanged) | TLS WebSocket connections reuse HttpStream::Tls in Phase 61. |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `sha1 0.10` | `sha1_smol` | sha1_smol is zero-dep, but sha1 0.10 is consistent with existing RustCrypto ecosystem (sha2, hmac, md-5). Consistency wins. |
| Hand-rolled frame parser | `tungstenite` crate | tungstenite is a full WebSocket library but pulls in many deps and assumes ownership of the connection. Snow needs tight control over the stream for actor integration. Hand-rolling the ~200-line frame codec is tractable and keeps the runtime dependency-light. |
| `byteorder` crate for big-endian u16/u64 | `u16::from_be_bytes` / `u64::from_be_bytes` | stdlib methods are sufficient. No need for byteorder crate. |

### Dependency Changes
```toml
# ADD to snow-rt/Cargo.toml:
sha1 = "0.10"

# ALREADY PRESENT (no changes):
base64 = "0.22"
# sha2 = "0.10"  -- sha1 and sha2 share the digest crate transitively
```

## Architecture Patterns

### Recommended Project Structure
```
crates/snow-rt/src/
  ws/
    mod.rs           # Module declarations, re-exports, WsOpcode enum
    handshake.rs     # HTTP upgrade validation + 101 Switching Protocols response
    frame.rs         # Frame parser (read) + frame writer (write) + masking
    close.rs         # Close handshake state machine + status codes
  http/
    server.rs        # UNCHANGED (but parse_request may be extracted for reuse)
```

### Pattern 1: WebSocket Upgrade Handshake
**What:** Validate an HTTP GET request as a WebSocket upgrade, compute Sec-WebSocket-Accept, write the 101 response.
**When to use:** Called once per connection before switching to frame-based I/O.
**Example:**
```rust
// Source: RFC 6455 Section 4.2.1 + Section 4.2.2
use sha1::{Sha1, Digest};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Compute the Sec-WebSocket-Accept value from the client's key.
fn compute_accept_key(client_key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(client_key.as_bytes());
    hasher.update(WS_GUID.as_bytes());
    BASE64.encode(hasher.finalize())
}

/// Validate upgrade request headers per RFC 6455 Section 4.2.1.
/// Returns the client's Sec-WebSocket-Key on success.
fn validate_upgrade_request(
    method: &str,
    headers: &[(String, String)],
) -> Result<String, &'static str> {
    if method != "GET" {
        return Err("upgrade requires GET method");
    }

    let mut has_upgrade = false;
    let mut has_connection = false;
    let mut ws_key = None;
    let mut ws_version = None;

    for (name, value) in headers {
        match name.to_ascii_lowercase().as_str() {
            "upgrade" => {
                if value.eq_ignore_ascii_case("websocket") {
                    has_upgrade = true;
                }
            }
            "connection" => {
                // Connection header may contain multiple tokens (e.g., "keep-alive, Upgrade")
                if value.to_ascii_lowercase().contains("upgrade") {
                    has_connection = true;
                }
            }
            "sec-websocket-key" => {
                ws_key = Some(value.clone());
            }
            "sec-websocket-version" => {
                ws_version = Some(value.clone());
            }
            _ => {}
        }
    }

    if !has_upgrade { return Err("missing Upgrade: websocket header"); }
    if !has_connection { return Err("missing Connection: Upgrade header"); }
    let key = ws_key.ok_or("missing Sec-WebSocket-Key header")?;
    let version = ws_version.ok_or("missing Sec-WebSocket-Version header")?;
    if version != "13" { return Err("unsupported Sec-WebSocket-Version (must be 13)"); }

    Ok(key)
}

/// Write the 101 Switching Protocols response.
fn write_upgrade_response<W: std::io::Write>(
    stream: &mut W,
    accept_key: &str,
) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\n\
         \r\n",
        accept_key
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}
```

### Pattern 2: Frame Parser (Variable-Length Header)
**What:** Parse a WebSocket frame from a byte stream, handling the 2-14 byte variable header.
**When to use:** Called in a loop after the handshake to read incoming frames.
**Example:**
```rust
// Source: RFC 6455 Section 5.2 (Base Framing Protocol)
use std::io::Read;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WsOpcode {
    Continuation = 0x0,
    Text = 0x1,
    Binary = 0x2,
    Close = 0x8,
    Ping = 0x9,
    Pong = 0xA,
}

pub struct WsFrame {
    pub fin: bool,
    pub opcode: WsOpcode,
    pub payload: Vec<u8>,
}

/// Parse one WebSocket frame from the stream.
fn read_frame<R: Read>(reader: &mut R) -> Result<WsFrame, String> {
    // Byte 0: FIN(1) RSV(3) Opcode(4)
    let mut header = [0u8; 2];
    reader.read_exact(&mut header).map_err(|e| format!("read frame header: {}", e))?;

    let fin = (header[0] & 0x80) != 0;
    let rsv = (header[0] >> 4) & 0x07;
    if rsv != 0 {
        return Err("non-zero RSV bits without negotiated extensions".to_string());
    }
    let opcode_byte = header[0] & 0x0F;

    let opcode = match opcode_byte {
        0x0 => WsOpcode::Continuation,
        0x1 => WsOpcode::Text,
        0x2 => WsOpcode::Binary,
        0x8 => WsOpcode::Close,
        0x9 => WsOpcode::Ping,
        0xA => WsOpcode::Pong,
        _ => return Err(format!("unknown opcode: 0x{:X}", opcode_byte)),
    };

    // Byte 1: MASK(1) Payload-Length(7)
    let masked = (header[1] & 0x80) != 0;
    let length_byte = header[1] & 0x7F;

    // Payload length: 3 encodings
    let payload_len: u64 = match length_byte {
        0..=125 => length_byte as u64,
        126 => {
            let mut buf = [0u8; 2];
            reader.read_exact(&mut buf).map_err(|e| format!("read 16-bit length: {}", e))?;
            u16::from_be_bytes(buf) as u64
        }
        127 => {
            let mut buf = [0u8; 8];
            reader.read_exact(&mut buf).map_err(|e| format!("read 64-bit length: {}", e))?;
            let len = u64::from_be_bytes(buf);
            if len >> 63 != 0 {
                return Err("MSB of 64-bit length must be 0".to_string());
            }
            len
        }
        _ => unreachable!(),
    };

    // Masking key (4 bytes, present only if MASK bit is set)
    let mask_key = if masked {
        let mut key = [0u8; 4];
        reader.read_exact(&mut key).map_err(|e| format!("read mask key: {}", e))?;
        Some(key)
    } else {
        None
    };

    // Read payload
    let mut payload = vec![0u8; payload_len as usize];
    if payload_len > 0 {
        reader.read_exact(&mut payload).map_err(|e| format!("read payload: {}", e))?;
    }

    // Unmask if needed (client-to-server frames MUST be masked)
    if let Some(key) = mask_key {
        apply_mask(&mut payload, &key);
    }

    Ok(WsFrame { fin, opcode, payload })
}
```

### Pattern 3: XOR Masking/Unmasking
**What:** Apply 4-byte XOR mask to WebSocket payload bytes.
**When to use:** Client-to-server frames are always masked. Server-to-client frames are never masked.
**Example:**
```rust
// Source: RFC 6455 Section 5.3
/// Apply or remove the 4-byte XOR mask on a payload.
/// The operation is symmetric: applying the mask twice returns the original.
fn apply_mask(payload: &mut [u8], mask_key: &[u8; 4]) {
    for (i, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask_key[i % 4];
    }
}
```

### Pattern 4: Frame Writer (Server-to-Client, Unmasked)
**What:** Write a WebSocket frame with the correct header encoding.
**When to use:** Server sends text/binary/close/pong frames to the client.
**Example:**
```rust
// Source: RFC 6455 Section 5.2
use std::io::Write;

fn write_frame<W: Write>(
    writer: &mut W,
    opcode: WsOpcode,
    payload: &[u8],
    fin: bool,
) -> Result<(), String> {
    // Byte 0: FIN + opcode
    let byte0 = if fin { 0x80 } else { 0x00 } | (opcode as u8);

    // Byte 1: MASK=0 + payload length (server MUST NOT mask)
    let len = payload.len();
    if len <= 125 {
        writer.write_all(&[byte0, len as u8])
            .map_err(|e| format!("write frame header: {}", e))?;
    } else if len <= 65535 {
        writer.write_all(&[byte0, 126])
            .map_err(|e| format!("write frame header: {}", e))?;
        writer.write_all(&(len as u16).to_be_bytes())
            .map_err(|e| format!("write 16-bit length: {}", e))?;
    } else {
        writer.write_all(&[byte0, 127])
            .map_err(|e| format!("write frame header: {}", e))?;
        writer.write_all(&(len as u64).to_be_bytes())
            .map_err(|e| format!("write 64-bit length: {}", e))?;
    }

    // Payload (no masking key for server-to-client)
    if !payload.is_empty() {
        writer.write_all(payload)
            .map_err(|e| format!("write payload: {}", e))?;
    }

    writer.flush().map_err(|e| format!("flush frame: {}", e))
}
```

### Pattern 5: Close Handshake State Machine
**What:** Manage the two-phase close process: send close frame, receive close frame, then close TCP.
**When to use:** When either side initiates connection close.
**Example:**
```rust
// Source: RFC 6455 Section 5.5.1 (Close)

/// Parse a close frame payload into status code and reason.
fn parse_close_payload(payload: &[u8]) -> (u16, String) {
    if payload.len() >= 2 {
        let code = u16::from_be_bytes([payload[0], payload[1]]);
        let reason = if payload.len() > 2 {
            String::from_utf8_lossy(&payload[2..]).to_string()
        } else {
            String::new()
        };
        (code, reason)
    } else {
        (1005, String::new()) // 1005 = no status code present
    }
}

/// Build a close frame payload from status code and optional reason.
fn build_close_payload(code: u16, reason: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(2 + reason.len());
    payload.extend_from_slice(&code.to_be_bytes());
    if !reason.is_empty() {
        let reason_bytes = reason.as_bytes();
        // Close frame payload max 125 bytes; status code takes 2, so reason max 123.
        let max_reason = 123.min(reason_bytes.len());
        payload.extend_from_slice(&reason_bytes[..max_reason]);
    }
    payload
}
```

### Pattern 6: HTTP 400 for Malformed Upgrades
**What:** Reject non-WebSocket or malformed upgrade requests with HTTP 400.
**When to use:** In the handshake validation, before switching to frame mode.
**Example:**
```rust
// Source: RFC 6455 Section 4.2.2 (server requirements)
fn write_bad_request<W: std::io::Write>(
    stream: &mut W,
    reason: &str,
) -> std::io::Result<()> {
    let body = format!("Bad Request: {}", reason);
    let response = format!(
        "HTTP/1.1 400 Bad Request\r\n\
         Content-Type: text/plain\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        body.len(),
        body,
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}
```

### Anti-Patterns to Avoid
- **Masking server-to-client frames:** RFC 6455 explicitly forbids this. The server MUST send unmasked frames. If a server masks frames, compliant clients MUST close the connection.
- **Accepting unmasked client frames:** Client-to-server frames MUST be masked. The server MUST close the connection (code 1002) if it receives an unmasked frame from a client.
- **Using BufReader for frame I/O:** BufReader buffers ahead, which is dangerous for WebSocket frames where you need exact byte control. Read the exact bytes needed for each header field. Use raw `Read::read_exact()` on the stream directly (not through BufReader). Note: BufReader is fine for the initial HTTP upgrade request (which IS line-based), but switch to raw reads after the upgrade.
- **Allocating payload buffer before reading length:** Read the 2-byte header first, determine the payload length encoding, read the extended length bytes, THEN allocate the payload buffer. Prevents over-allocation on short frames.
- **Treating continuation opcode as a standalone frame:** Opcode 0x0 (continuation) is only valid when a previous frame had FIN=0. Phase 59 does not implement fragmentation reassembly (that is Phase 61 FRAG-01), but the opcode should still be recognized so it does not trigger the unknown-opcode close.
- **Forgetting big-endian for extended lengths:** WebSocket uses network byte order (big-endian) for 16-bit and 64-bit payload lengths and for the close status code. Rust's `u16::from_be_bytes` and `u64::from_be_bytes` handle this correctly. Do NOT use `from_le_bytes`.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| SHA-1 hash | Custom SHA-1 | `sha1 0.10` crate | SHA-1 is 600+ lines of crypto. sha1 0.10 is from the same RustCrypto org as the existing sha2 0.10 dep. Same Digest trait, zero learning curve. |
| Base64 encoding | Custom base64 | `base64 0.22` crate | Already a dependency. Handles padding, URL-safe variants, etc. |
| UTF-8 validation | Custom UTF-8 checker | `std::str::from_utf8()` | Rust stdlib's UTF-8 validator is battle-tested and fast. |
| HTTP/1.1 request parsing | New HTTP parser | Reuse existing `parse_request` from `http/server.rs` | The upgrade request is a standard HTTP GET. No need to duplicate parsing. |

**Key insight:** The WebSocket protocol is a thin framing layer on top of TCP. The frame codec itself is only ~200 lines of Rust. The complexity is in correctness (byte ordering, masking direction, payload length encoding, close handshake state) rather than volume. The only new dependency (`sha1`) is a one-liner addition.

## Common Pitfalls

### Pitfall 1: Masking Direction Confusion
**What goes wrong:** Server masks outgoing frames OR server fails to unmask incoming frames, causing garbled data.
**Why it happens:** The masking rules are asymmetric: client-to-server is ALWAYS masked, server-to-client is NEVER masked. This is easy to confuse, especially when writing test helpers that simulate both sides.
**How to avoid:** Encode the rule in the type system or function names: `read_client_frame` always unmasks; `write_server_frame` never masks. Test with a real WebSocket client (e.g., `websocat` or a browser) to verify interop.
**Warning signs:** Browser console shows "invalid frame" or garbled text. `websocat` shows binary garbage instead of text.

### Pitfall 2: BufReader Consuming Beyond the HTTP Upgrade
**What goes wrong:** If the HTTP upgrade request is read through a `BufReader`, the BufReader may buffer the first WebSocket frame bytes beyond the `\r\n\r\n`. When you switch to raw frame reading, those bytes are lost.
**Why it happens:** BufReader reads in 8KB chunks. If the client sends the upgrade request AND the first WebSocket frame in the same TCP segment, the BufReader will consume both.
**How to avoid:** After reading the HTTP upgrade, check `BufReader::buffer()` for leftover bytes. Prepend any leftover bytes to the frame reader. Alternatively, use a `Cursor` wrapping any remainder + the underlying stream. The cleanest approach: read the upgrade headers via BufReader, then call `into_inner()` to get the underlying stream back -- but note that any buffered-but-unread bytes in the BufReader are lost! The safest approach is to check `BufReader::buffer()` and carry forward any remaining bytes.
**Warning signs:** First WebSocket frame after handshake is corrupted or missing. Subsequent frames work fine.

### Pitfall 3: Payload Length Encoding for Close Frames
**What goes wrong:** Close frame payload (status code + reason) exceeds 125 bytes, violating RFC 6455 control frame size limit.
**Why it happens:** Control frames (close, ping, pong) MUST have payload <= 125 bytes. The close frame has 2 bytes for status code + up to 123 bytes for reason. If the reason string is too long, the frame is invalid.
**How to avoid:** Truncate close reasons to 123 bytes. Always validate control frame payloads <= 125 bytes when reading.
**Warning signs:** Clients reject close frames. The close handshake never completes.

### Pitfall 4: Forgetting to Validate UTF-8 for Text Frames
**What goes wrong:** Text frame (opcode 0x1) contains invalid UTF-8, but server delivers raw bytes to the application.
**Why it happens:** RFC 6455 requires text frames to contain valid UTF-8. Server implementations must validate this.
**How to avoid:** After unmasking a text frame, call `std::str::from_utf8(&payload)`. If it fails, send a close frame with code 1007 (Invalid frame payload data).
**Warning signs:** Browsers close connections when receiving text frames with binary data.

### Pitfall 5: Close Handshake Deadlock
**What goes wrong:** Both sides send close frames simultaneously, but neither side reads the other's close frame because they stopped reading after sending their own close.
**Why it happens:** After sending a close frame, some implementations stop reading, waiting for TCP close. But RFC 6455 says the receiving endpoint should still read and process the peer's close frame.
**How to avoid:** After sending a close frame, continue reading until you receive the peer's close frame (or a read timeout/error). Only then close the TCP connection. Implement a `closing` state flag that prevents sending data frames but still processes incoming close frames.
**Warning signs:** Connections hang in a half-closed state. TCP FIN-WAIT timers accumulate.

### Pitfall 6: 64-bit Payload Length MSB
**What goes wrong:** A malicious client sends a frame with the MSB of the 64-bit payload length set, indicating a negative length or astronomically large allocation.
**Why it happens:** RFC 6455 says the most significant bit of the 64-bit length MUST be 0. Failing to check this can cause integer overflow or out-of-memory panics.
**How to avoid:** After reading the 8-byte length, check `if len >> 63 != 0 { return Err(...) }`. Also apply a reasonable max payload size limit (Phase 61 caps at 16MB, but even in Phase 59 a sanity limit like 64MB prevents OOM).
**Warning signs:** Server panics or hangs on large frames. Memory exhaustion.

### Pitfall 7: Network Byte Order for Extended Lengths and Close Codes
**What goes wrong:** Payload lengths or close status codes are read/written in little-endian instead of big-endian (network byte order).
**Why it happens:** x86 is little-endian natively. Using `from_ne_bytes` or `from_le_bytes` instead of `from_be_bytes` silently produces wrong values that only manifest as garbled frame sizes.
**How to avoid:** Always use `from_be_bytes` / `to_be_bytes` for multi-byte WebSocket protocol fields. This applies to: 16-bit extended payload length, 64-bit extended payload length, and 2-byte close status code.
**Warning signs:** Frame sizes look random. A 256-byte payload is read as 1 byte (0x0100 vs 0x0001).

## Code Examples

Verified patterns from official sources:

### Sec-WebSocket-Accept Computation (RFC 6455 Section 4.2.2)
```rust
// Source: RFC 6455 Section 4.2.2, Example value verified
use sha1::{Sha1, Digest};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

fn compute_accept_key(client_key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(client_key.as_bytes());
    hasher.update(WS_GUID.as_bytes());
    BASE64.encode(hasher.finalize())
}

#[test]
fn test_accept_key_rfc_example() {
    // RFC 6455 Section 4.2.2 example
    let key = "dGhlIHNhbXBsZSBub25jZQ==";
    let accept = compute_accept_key(key);
    assert_eq!(accept, "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
}
```

### XOR Masking (RFC 6455 Section 5.3)
```rust
// Source: RFC 6455 Section 5.3
fn apply_mask(payload: &mut [u8], mask_key: &[u8; 4]) {
    for (i, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask_key[i % 4];
    }
}

#[test]
fn test_mask_roundtrip() {
    let original = b"Hello".to_vec();
    let key = [0x37, 0xfa, 0x21, 0x3d];
    let mut masked = original.clone();
    apply_mask(&mut masked, &key);
    assert_ne!(masked, original); // masked is different
    apply_mask(&mut masked, &key); // unmask
    assert_eq!(masked, original); // back to original
}
```

### Frame Read with All Three Length Encodings
```rust
// Source: RFC 6455 Section 5.2
// 7-bit: payload_len 0-125, header is 2 bytes (+ 4 mask)
// 16-bit: payload_len == 126, next 2 bytes are actual length, header is 4 bytes (+ 4 mask)
// 64-bit: payload_len == 127, next 8 bytes are actual length, header is 10 bytes (+ 4 mask)

#[test]
fn test_parse_7bit_length() {
    // A masked text frame "Hi" (2 bytes) from client
    // FIN=1, opcode=0x1 (text), MASK=1, len=2, mask_key=[0,0,0,0], payload="Hi"
    let frame_bytes: Vec<u8> = vec![
        0x81,       // FIN=1, opcode=0x1
        0x82,       // MASK=1, len=2
        0, 0, 0, 0, // mask key (all zeros = payload unchanged)
        b'H', b'i', // payload
    ];
    let mut cursor = std::io::Cursor::new(frame_bytes);
    let frame = read_frame(&mut cursor).unwrap();
    assert!(frame.fin);
    assert_eq!(frame.opcode, WsOpcode::Text);
    assert_eq!(frame.payload, b"Hi");
}
```

### Close Frame Construction
```rust
// Source: RFC 6455 Section 5.5.1
#[test]
fn test_close_frame_normal() {
    let payload = build_close_payload(1000, "normal closure");
    assert_eq!(payload[0], 0x03); // 1000 >> 8
    assert_eq!(payload[1], 0xE8); // 1000 & 0xFF
    assert_eq!(&payload[2..], b"normal closure");
}
```

### HTTP 400 Response for Bad Upgrade
```rust
// Source: RFC 6455 Section 4.2.2 (opening handshake failure)
#[test]
fn test_reject_missing_upgrade_header() {
    let headers = vec![
        ("Connection".to_string(), "Upgrade".to_string()),
        ("Sec-WebSocket-Key".to_string(), "dGhlIHNhbXBsZSBub25jZQ==".to_string()),
        ("Sec-WebSocket-Version".to_string(), "13".to_string()),
        // Missing: Upgrade: websocket
    ];
    let result = validate_upgrade_request("GET", &headers);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Upgrade"));
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `sha-1` crate (hyphenated) | `sha1` crate (no hyphen) | sha1 v0.10.0 (2022) | sha-1 is deprecated. Use sha1 0.10 for new code. Same Digest trait. |
| WebSocket draft protocols | RFC 6455 (Sec-WebSocket-Version: 13) | Dec 2011 (stable) | Version 13 is the only version in use. All browsers and clients use it. |
| Custom crypto providers for rustls | `CryptoProvider::install_default(ring)` | rustls 0.22 (Dec 2023) | Already handled in Phase 55's snow_rt_init(). No action needed. |

**Deprecated/outdated:**
- `sha-1` crate: Deprecated, use `sha1` 0.10+.
- Sec-WebSocket-Version other than 13: All pre-RFC versions are obsolete. Only check for `13`.
- Hixie-76 WebSocket protocol: Ancient pre-RFC protocol. Not supported by any modern client. Do not implement.

## Open Questions

1. **BufReader handoff between HTTP and WebSocket**
   - What we know: The HTTP upgrade request is read through a BufReader (same as parse_request in server.rs). After the 101 response, the connection switches to WebSocket frame I/O.
   - What's unclear: Whether the BufReader buffers bytes beyond the HTTP headers that are actually the start of the first WebSocket frame.
   - Recommendation: After reading the upgrade request, extract any buffered bytes from the BufReader via `buffer()`. If non-empty, wrap them in a `Cursor` chained with the raw stream for subsequent frame reads. In practice, most clients wait for the 101 response before sending frames, so this is unlikely but must be handled for correctness. Alternatively, simply use `into_parts()` to get both the buffered bytes and the inner stream.

2. **Where to place the handshake entry point for Phase 60 integration**
   - What we know: Phase 59 builds the protocol layer. Phase 60 wires it into the actor system with `Ws.serve`. The handshake function needs to be callable from Phase 60's accept loop.
   - What's unclear: The exact signature Phase 60 will need.
   - Recommendation: Design the handshake as a pure function `perform_upgrade(stream: &mut impl Read + Write) -> Result<(), UpgradeError>` that reads the HTTP request, validates it, and writes the 101 response. Phase 60 can call this from its connection handler actor. This keeps Phase 59 free of actor system dependencies.

3. **Continuation frame handling in Phase 59**
   - What we know: Opcode 0x0 (continuation) is for fragmented messages. Phase 59 requirements do NOT include fragmentation reassembly (that is Phase 61 FRAG-01/FRAG-02/FRAG-03).
   - What's unclear: Should Phase 59 reject continuation frames, or just parse them and pass them through?
   - Recommendation: The frame parser should recognize opcode 0x0 as valid (do not trigger close code 1002). But since Phase 59 does not reassemble fragments, the frame reader should just return the frame as-is with `WsOpcode::Continuation`. Phase 60/61 will handle reassembly semantics. This avoids breaking changes when fragmentation support lands in Phase 61.

4. **Maximum payload size limit for Phase 59**
   - What we know: Phase 61 introduces a 16MB max message size (FRAG-03). Phase 59 has no explicit size limit requirement.
   - What's unclear: Whether to add a safety limit now or defer.
   - Recommendation: Add a generous safety limit (e.g., 64MB) to prevent OOM from malicious 64-bit length values. This is not a business requirement but a runtime safety measure. Phase 61 will tighten it to 16MB.

## Sources

### Primary (HIGH confidence)
- **RFC 6455** (The WebSocket Protocol) -- https://datatracker.ietf.org/doc/html/rfc6455
  - Section 4.2.1: Client opening handshake requirements
  - Section 4.2.2: Server opening handshake response + Sec-WebSocket-Accept computation
  - Section 5.2: Base framing protocol (header format, opcodes, length encoding)
  - Section 5.3: Client-to-server masking algorithm
  - Section 5.5.1: Close control frame format and status codes
  - Section 7.4.1: Defined close status codes

- **Snow codebase** (direct reading):
  - `crates/snow-rt/src/http/server.rs` -- Existing HTTP parser (parse_request), HttpStream enum, connection handler pattern, actor spawn, catch_unwind isolation
  - `crates/snow-rt/src/http/mod.rs` -- Module structure pattern to follow for ws/mod.rs
  - `crates/snow-rt/src/actor/mod.rs` -- Actor send/receive, mailbox, type_tag system (EXIT_SIGNAL_TAG = u64::MAX)
  - `crates/snow-rt/src/actor/process.rs` -- Process/Message/MessageBuffer types
  - `crates/snow-rt/src/db/pg.rs` -- sha2 Digest usage pattern, base64 ENGINE usage pattern
  - `crates/snow-rt/Cargo.toml` -- Existing deps: sha2 0.10, base64 0.22, rustls 0.23

- **sha1 crate docs** -- https://docs.rs/sha1/0.10.6/sha1/
  - API: `Sha1::new()`, `hasher.update()`, `hasher.finalize()` -- identical to sha2 Digest trait pattern

- **base64 crate docs** -- https://docs.rs/base64/0.22/base64/
  - API: `base64::engine::general_purpose::STANDARD.encode()` -- already used in pg.rs

### Secondary (MEDIUM confidence)
- **RustCrypto/hashes GitHub** -- https://github.com/RustCrypto/hashes
  - Confirmed sha1 0.10 is from the same org as sha2 0.10 and uses the shared `digest` trait crate

### Tertiary (LOW confidence)
- None -- all critical claims verified against RFC spec, crate docs, or existing codebase.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- sha1 0.10 API verified against docs.rs, compatible with existing sha2/base64 deps. Only one new dependency line.
- Architecture: HIGH -- Frame format fully specified by RFC 6455. ws/ module structure follows proven http/ module pattern. Code examples tested against RFC examples.
- Pitfalls: HIGH -- Masking direction, BufReader handoff, byte ordering, and close handshake semantics are well-documented RFC requirements. Verified against actual protocol specification.

**Research date:** 2026-02-12
**Valid until:** 2026-03-12 (extremely stable domain -- RFC 6455 is from 2011 and has not changed)
