//! WebSocket HTTP upgrade handshake (RFC 6455 Section 4.2).
//!
//! Validates the client's HTTP upgrade request, computes the
//! `Sec-WebSocket-Accept` response header, and writes the `101 Switching
//! Protocols` response (or `400 Bad Request` on failure).
//!
//! - [`perform_upgrade`]: Main entry point -- reads HTTP request, validates, writes response
//! - [`compute_accept_key`]: SHA-1 + Base64 computation per RFC 6455 Section 4.2.2
//! - [`validate_upgrade_request`]: Header validation against RFC requirements
//! - [`write_upgrade_response`]: Writes the 101 Switching Protocols response
//! - [`write_bad_request`]: Writes the 400 Bad Request response

use std::io::{BufRead, BufReader, Read, Write};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use sha1::{Digest, Sha1};

/// RFC 6455 magic GUID concatenated with the client key for Sec-WebSocket-Accept.
const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

pub(crate) const MAX_HANDSHAKE_BYTES: usize = 16 * 1024;

pub(crate) struct ParsedUpgradeRequest {
    pub(crate) path: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) response: Vec<u8>,
    pub(crate) consumed: usize,
}

fn complete_headers(bytes: &[u8]) -> Result<Option<usize>, String> {
    if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
        let consumed = position + 4;
        if consumed > MAX_HANDSHAKE_BYTES {
            return Err(format!(
                "WebSocket handshake headers exceed {MAX_HANDSHAKE_BYTES} bytes"
            ));
        }
        Ok(Some(consumed))
    } else if bytes.len() > MAX_HANDSHAKE_BYTES {
        Err(format!(
            "WebSocket handshake headers exceed {MAX_HANDSHAKE_BYTES} bytes"
        ))
    } else {
        Ok(None)
    }
}

pub(crate) fn parse_upgrade_request_bytes(
    bytes: &[u8],
) -> Result<Option<ParsedUpgradeRequest>, String> {
    let Some(consumed) = complete_headers(bytes)? else {
        return Ok(None);
    };
    let request = std::str::from_utf8(&bytes[..consumed])
        .map_err(|_| "WebSocket handshake is not valid UTF-8".to_string())?;
    let mut lines = request.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| "missing WebSocket request line".to_string())?;
    let mut parts = request_line.split_ascii_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    let version = parts.next().unwrap_or_default();
    if path.is_empty() || version != "HTTP/1.1" || parts.next().is_some() {
        return Err(format!("malformed WebSocket request line: {request_line}"));
    }
    let headers = lines
        .take_while(|line| !line.is_empty())
        .map(|line| {
            line.split_once(':')
                .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
                .ok_or_else(|| format!("malformed WebSocket header: {line}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let client_key = validate_upgrade_request(method, &headers).map_err(str::to_string)?;
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        compute_accept_key(&client_key)
    )
    .into_bytes();
    Ok(Some(ParsedUpgradeRequest {
        path: path.to_string(),
        headers,
        response,
        consumed,
    }))
}

pub(crate) fn parse_upgrade_response_bytes(
    bytes: &[u8],
    client_key: &str,
) -> Result<Option<usize>, String> {
    let Some(consumed) = complete_headers(bytes)? else {
        return Ok(None);
    };
    let response = std::str::from_utf8(&bytes[..consumed])
        .map_err(|_| "WebSocket handshake is not valid UTF-8".to_string())?;
    let mut lines = response.split("\r\n");
    let status = lines.next().unwrap_or_default();
    let mut status_parts = status.split_ascii_whitespace();
    if !matches!(status_parts.next(), Some("HTTP/1.1" | "HTTP/1.0"))
        || status_parts.next() != Some("101")
    {
        return Err(format!("WebSocket upgrade rejected: {status}"));
    }
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim()))
        .collect::<Vec<_>>();
    let has_token = |name: &str, expected: &str| {
        headers.iter().any(|(header_name, value)| {
            header_name == name
                && value
                    .split(',')
                    .any(|token| token.trim().eq_ignore_ascii_case(expected))
        })
    };
    if !has_token("upgrade", "websocket") || !has_token("connection", "upgrade") {
        return Err("WebSocket upgrade response is missing required headers".to_string());
    }
    let expected = compute_accept_key(client_key);
    if !headers
        .iter()
        .any(|(name, value)| name == "sec-websocket-accept" && *value == expected.as_str())
    {
        return Err("WebSocket upgrade response has an invalid accept key".to_string());
    }
    Ok(Some(consumed))
}

/// Compute the `Sec-WebSocket-Accept` value per RFC 6455 Section 4.2.2.
///
/// Concatenates `client_key` + [`WS_GUID`], SHA-1 hashes, then Base64 encodes.
pub fn compute_accept_key(client_key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(client_key.as_bytes());
    hasher.update(WS_GUID.as_bytes());
    let hash = hasher.finalize();
    BASE64.encode(hash)
}

/// Validate an HTTP upgrade request per RFC 6455 Section 4.2.1.
///
/// Returns `Ok(client_key)` if all required headers are present and valid,
/// or `Err(reason)` describing the first validation failure.
pub fn validate_upgrade_request(
    method: &str,
    headers: &[(String, String)],
) -> Result<String, &'static str> {
    // Method must be GET
    if !method.eq_ignore_ascii_case("GET") {
        return Err("method must be GET");
    }

    // Helper: find a header value by case-insensitive name
    let find_header = |name: &str| -> Option<&str> {
        headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    };

    // Upgrade header must contain "websocket" (case-insensitive value)
    match find_header("Upgrade") {
        Some(v) if v.to_ascii_lowercase().contains("websocket") => {}
        _ => return Err("missing or invalid Upgrade header"),
    }

    // Connection header must contain "upgrade" (case-insensitive, may have multiple tokens)
    match find_header("Connection") {
        Some(v) if v.to_ascii_lowercase().contains("upgrade") => {}
        _ => return Err("missing or invalid Connection header"),
    }

    // Sec-WebSocket-Key must be present
    let client_key = match find_header("Sec-WebSocket-Key") {
        Some(k) => k.to_string(),
        None => return Err("missing Sec-WebSocket-Key header"),
    };

    // Sec-WebSocket-Version must be "13"
    match find_header("Sec-WebSocket-Version") {
        Some("13") => {}
        _ => return Err("missing or invalid Sec-WebSocket-Version (must be 13)"),
    }

    Ok(client_key)
}

/// Write the `101 Switching Protocols` response to the stream.
pub fn write_upgrade_response<W: Write>(stream: &mut W, accept_key: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\n\
         \r\n",
        accept_key
    )?;
    stream.flush()
}

/// Write a `400 Bad Request` response with the given reason.
pub fn write_bad_request<W: Write>(stream: &mut W, reason: &str) -> std::io::Result<()> {
    let body = format!("Bad Request: {}", reason);
    write!(
        stream,
        "HTTP/1.1 400 Bad Request\r\n\
         Content-Type: text/plain\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        body.len(),
        body
    )?;
    stream.flush()
}

/// Perform the WebSocket upgrade handshake on a raw stream.
///
/// Reads the HTTP upgrade request, validates it, and writes either a
/// `101 Switching Protocols` or `400 Bad Request` response. After a
/// successful upgrade, the stream is ready for WebSocket frame I/O.
///
/// This is the main entry point that Phase 60 will call.
///
/// # BufReader safety note
///
/// The BufReader borrows `stream` for header parsing. After headers are read,
/// the borrow ends and the caller resumes raw stream access for frame I/O.
/// This is safe because RFC 6455 clients do not send frames before receiving
/// the 101 response. We verify the buffer is empty as a sanity check.
pub fn perform_upgrade<S: Read + Write>(
    stream: &mut S,
) -> Result<(String, Vec<(String, String)>), String> {
    let mut reader = BufReader::new(&mut *stream);

    // 1. Read request line: "GET /path HTTP/1.1\r\n"
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|e| format!("read request line: {}", e))?;

    let request_line_trimmed = request_line.trim_end();
    let parts: Vec<&str> = request_line_trimmed.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err(format!("malformed request line: {}", request_line_trimmed));
    }
    let method = parts[0];
    let path = parts[1].to_string();

    // 2. Read headers until blank line
    let mut headers: Vec<(String, String)> = Vec::new();
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| format!("read header: {}", e))?;

        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }

    // Sanity check: BufReader should not have buffered extra bytes
    if !reader.buffer().is_empty() {
        eprintln!(
            "[mesh-rt] warning: {} bytes buffered beyond HTTP headers during WebSocket upgrade",
            reader.buffer().len()
        );
    }

    // Drop the reader to release the borrow on stream
    drop(reader);

    // 3. Validate and respond
    match validate_upgrade_request(method, &headers) {
        Ok(client_key) => {
            let accept_key = compute_accept_key(&client_key);
            write_upgrade_response(stream, &accept_key)
                .map_err(|e| format!("write upgrade response: {}", e))?;
            Ok((path, headers))
        }
        Err(reason) => {
            let _ = write_bad_request(stream, reason);
            Err(reason.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn incremental_server_handshake_waits_for_terminator_and_preserves_remainder() {
        let request = b"GET /feed?after=4 HTTP/1.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n\x81\x00";
        assert!(parse_upgrade_request_bytes(&request[..20])
            .unwrap()
            .is_none());

        let parsed = parse_upgrade_request_bytes(request).unwrap().unwrap();
        assert_eq!(parsed.path, "/feed?after=4");
        assert_eq!(&request[parsed.consumed..], &[0x81, 0x00]);
        assert!(parsed.response.starts_with(b"HTTP/1.1 101"));
    }

    #[test]
    fn incremental_client_handshake_validates_accept_key_and_header_limit() {
        let response = b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n";
        assert!(
            parse_upgrade_response_bytes(&response[..12], "dGhlIHNhbXBsZSBub25jZQ==")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            parse_upgrade_response_bytes(response, "dGhlIHNhbXBsZSBub25jZQ==")
                .unwrap()
                .unwrap(),
            response.len()
        );
        assert!(parse_upgrade_response_bytes(response, "wrong").is_err());
        assert!(parse_upgrade_request_bytes(&vec![b'x'; MAX_HANDSHAKE_BYTES + 1]).is_err());
    }

    #[test]
    fn test_accept_key_rfc_example() {
        // RFC 6455 Section 4.2.2 test vector
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let accept = compute_accept_key(key);
        assert_eq!(accept, "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn test_validate_valid_upgrade() {
        let headers = vec![
            ("Upgrade".to_string(), "websocket".to_string()),
            ("Connection".to_string(), "Upgrade".to_string()),
            (
                "Sec-WebSocket-Key".to_string(),
                "dGhlIHNhbXBsZSBub25jZQ==".to_string(),
            ),
            ("Sec-WebSocket-Version".to_string(), "13".to_string()),
        ];
        let result = validate_upgrade_request("GET", &headers);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "dGhlIHNhbXBsZSBub25jZQ==");
    }

    #[test]
    fn test_validate_missing_upgrade_header() {
        let headers = vec![
            ("Connection".to_string(), "Upgrade".to_string()),
            (
                "Sec-WebSocket-Key".to_string(),
                "dGhlIHNhbXBsZSBub25jZQ==".to_string(),
            ),
            ("Sec-WebSocket-Version".to_string(), "13".to_string()),
        ];
        let result = validate_upgrade_request("GET", &headers);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Upgrade"));
    }

    #[test]
    fn test_validate_missing_connection_header() {
        let headers = vec![
            ("Upgrade".to_string(), "websocket".to_string()),
            (
                "Sec-WebSocket-Key".to_string(),
                "dGhlIHNhbXBsZSBub25jZQ==".to_string(),
            ),
            ("Sec-WebSocket-Version".to_string(), "13".to_string()),
        ];
        let result = validate_upgrade_request("GET", &headers);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Connection"));
    }

    #[test]
    fn test_validate_missing_key() {
        let headers = vec![
            ("Upgrade".to_string(), "websocket".to_string()),
            ("Connection".to_string(), "Upgrade".to_string()),
            ("Sec-WebSocket-Version".to_string(), "13".to_string()),
        ];
        let result = validate_upgrade_request("GET", &headers);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Sec-WebSocket-Key"));
    }

    #[test]
    fn test_validate_wrong_version() {
        let headers = vec![
            ("Upgrade".to_string(), "websocket".to_string()),
            ("Connection".to_string(), "Upgrade".to_string()),
            (
                "Sec-WebSocket-Key".to_string(),
                "dGhlIHNhbXBsZSBub25jZQ==".to_string(),
            ),
            ("Sec-WebSocket-Version".to_string(), "8".to_string()),
        ];
        let result = validate_upgrade_request("GET", &headers);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Version"));
    }

    #[test]
    fn test_validate_wrong_method() {
        let headers = vec![
            ("Upgrade".to_string(), "websocket".to_string()),
            ("Connection".to_string(), "Upgrade".to_string()),
            (
                "Sec-WebSocket-Key".to_string(),
                "dGhlIHNhbXBsZSBub25jZQ==".to_string(),
            ),
            ("Sec-WebSocket-Version".to_string(), "13".to_string()),
        ];
        let result = validate_upgrade_request("POST", &headers);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("GET"));
    }

    #[test]
    fn test_perform_upgrade_success() {
        // Simulate a full upgrade: write a valid HTTP request, wrap in Cursor,
        // call perform_upgrade, and check the written output.
        let request = "GET /ws HTTP/1.1\r\n\
                        Upgrade: websocket\r\n\
                        Connection: Upgrade\r\n\
                        Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                        Sec-WebSocket-Version: 13\r\n\
                        \r\n";

        let buf = Cursor::new(request.as_bytes().to_vec());
        // We need a Read+Write stream. Cursor<Vec<u8>> is Read+Write,
        // but we need to read from the request and capture writes separately.
        // Use a helper struct that reads from one buffer and writes to another.
        struct TestStream {
            read_buf: Cursor<Vec<u8>>,
            write_buf: Vec<u8>,
        }

        impl Read for TestStream {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                self.read_buf.read(buf)
            }
        }

        impl Write for TestStream {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.write_buf.write(buf)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let mut stream = TestStream {
            read_buf: buf,
            write_buf: Vec::new(),
        };

        let result = perform_upgrade(&mut stream);
        assert!(result.is_ok(), "upgrade should succeed, got: {:?}", result);

        let (path, headers) = result.unwrap();
        assert_eq!(path, "/ws", "path should be /ws");
        assert!(!headers.is_empty(), "headers should not be empty");

        let response = String::from_utf8_lossy(&stream.write_buf);
        assert!(
            response.contains("101 Switching Protocols"),
            "response should contain 101, got: {}",
            response
        );
        assert!(
            response.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="),
            "response should contain correct Sec-WebSocket-Accept, got: {}",
            response
        );
    }

    #[test]
    fn test_perform_upgrade_bad_request() {
        // Simulate a non-upgrade GET request (missing WebSocket headers)
        let request = "GET / HTTP/1.1\r\n\
                        Host: example.com\r\n\
                        \r\n";

        struct TestStream {
            read_buf: Cursor<Vec<u8>>,
            write_buf: Vec<u8>,
        }

        impl Read for TestStream {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                self.read_buf.read(buf)
            }
        }

        impl Write for TestStream {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.write_buf.write(buf)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let mut stream = TestStream {
            read_buf: Cursor::new(request.as_bytes().to_vec()),
            write_buf: Vec::new(),
        };

        let result = perform_upgrade(&mut stream);
        assert!(
            result.is_err(),
            "upgrade should fail for non-upgrade request"
        );

        let response = String::from_utf8_lossy(&stream.write_buf);
        assert!(
            response.contains("400 Bad Request"),
            "response should contain 400, got: {}",
            response
        );
    }
}
