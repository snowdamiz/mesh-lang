//! WebSocket HTTP upgrade handshake (RFC 6455 Section 4.2).
//!
//! Parses and validates the client's HTTP upgrade request and the server's
//! answer, as the reactor reads them, and computes the
//! `Sec-WebSocket-Accept` key.
//!
//! - [`compute_accept_key`]: SHA-1 + Base64 computation per RFC 6455 Section 4.2.2
//! - [`validate_upgrade_request`]: Header validation against RFC requirements

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
            header_name == name && header_list_has_token(value, expected)
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

/// Whether a comma-separated header list (`Connection`, `Upgrade`) contains
/// `expected` as a whole, case-insensitive token. Substring matches such as
/// `notwebsocket` or `upgraded` are not upgrades.
fn header_list_has_token(value: &str, expected: &str) -> bool {
    value
        .split(',')
        .any(|token| token.trim().eq_ignore_ascii_case(expected))
}

/// Length in bytes of a decoded `Sec-WebSocket-Key` (RFC 6455 Section 4.2.1).
const WS_KEY_NONCE_BYTES: usize = 16;

/// Whether `key` is the base64 encoding of a 16-byte nonce, as RFC 6455
/// requires. The accept key is derived from the exact text the client sent,
/// so anything else (empty, non-base64, wrong length, embedded controls) is
/// refused rather than echoed back into the 101 response.
fn is_valid_client_key(key: &str) -> bool {
    key.len() == 24
        && key.is_ascii()
        && BASE64
            .decode(key)
            .is_ok_and(|nonce| nonce.len() == WS_KEY_NONCE_BYTES)
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

    // Upgrade header must list the "websocket" protocol token
    match find_header("Upgrade") {
        Some(v) if header_list_has_token(v, "websocket") => {}
        _ => return Err("missing or invalid Upgrade header"),
    }

    // Connection header must list the "upgrade" option token (it may also
    // carry others, e.g. `keep-alive, Upgrade`)
    match find_header("Connection") {
        Some(v) if header_list_has_token(v, "upgrade") => {}
        _ => return Err("missing or invalid Connection header"),
    }

    // Sec-WebSocket-Key must be present and decode to a 16-byte nonce
    let client_key = match find_header("Sec-WebSocket-Key") {
        Some(k) if is_valid_client_key(k) => k.to_string(),
        Some(_) => return Err("invalid Sec-WebSocket-Key header (must be base64 of 16 bytes)"),
        None => return Err("missing Sec-WebSocket-Key header"),
    };

    // Sec-WebSocket-Version must be "13"
    match find_header("Sec-WebSocket-Version") {
        Some("13") => {}
        _ => return Err("missing or invalid Sec-WebSocket-Version (must be 13)"),
    }

    Ok(client_key)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Headers whose end comes past the limit are refused as those that do
    /// not end within it are; an answer without the upgrade headers is
    /// refused.
    #[test]
    fn handshakes_past_the_limit_or_without_upgrade_headers_are_refused() {
        let long = [
            b"GET / HTTP/1.1\r\nX-Pad: ".as_slice(),
            &vec![b'a'; MAX_HANDSHAKE_BYTES],
            b"\r\n\r\n",
        ]
        .concat();
        assert_eq!(
            parse_upgrade_request_bytes(&long).err(),
            Some(format!(
                "WebSocket handshake headers exceed {MAX_HANDSHAKE_BYTES} bytes"
            ))
        );
        let bare = b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\n\r\n";
        assert_eq!(
            parse_upgrade_response_bytes(bare, "dGhlIHNhbXBsZSBub25jZQ==").err(),
            Some("WebSocket upgrade response is missing required headers".to_string())
        );
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

    fn valid_upgrade_headers() -> Vec<(String, String)> {
        vec![
            ("Upgrade".to_string(), "websocket".to_string()),
            ("Connection".to_string(), "Upgrade".to_string()),
            (
                "Sec-WebSocket-Key".to_string(),
                "dGhlIHNhbXBsZSBub25jZQ==".to_string(),
            ),
            ("Sec-WebSocket-Version".to_string(), "13".to_string()),
        ]
    }

    fn with_header(name: &str, value: &str) -> Vec<(String, String)> {
        valid_upgrade_headers()
            .into_iter()
            .map(|(header, current)| {
                if header.eq_ignore_ascii_case(name) {
                    (header, value.to_string())
                } else {
                    (header, current)
                }
            })
            .collect()
    }

    #[test]
    fn upgrade_validation_matches_whole_tokens_not_substrings() {
        for (name, value) in [
            ("Upgrade", "notwebsocket"),
            ("Upgrade", "websockets"),
            ("Upgrade", "web socket"),
            ("Upgrade", ""),
            ("Connection", "notupgrade"),
            ("Connection", "upgraded"),
            ("Connection", "keep-alive"),
            ("Connection", ""),
        ] {
            assert!(
                validate_upgrade_request("GET", &with_header(name, value)).is_err(),
                "{name}: {value:?} must be rejected"
            );
        }

        for (name, value) in [
            ("Upgrade", "WebSocket"),
            ("Upgrade", "h2c, websocket"),
            ("Connection", "keep-alive, Upgrade"),
            ("Connection", "Upgrade,keep-alive"),
            ("Connection", "UPGRADE"),
        ] {
            assert!(
                validate_upgrade_request("GET", &with_header(name, value)).is_ok(),
                "{name}: {value:?} must be accepted"
            );
        }
    }

    #[test]
    fn upgrade_validation_requires_a_base64_16_byte_key() {
        for key in [
            "",
            "not base64!",
            "dGhlIHNhbXBsZSBub25jZQ",       // missing padding
            "dGhlIHNhbXBsZSBub25jZQ=",      // wrong padding
            "dGhlIHNhbXBsZQ==",             // decodes to 10 bytes
            "dGhlIHNhbXBsZSBub25jZSB4eHg=", // decodes to 20 bytes
            "dGhlIHNhbXBsZSBub25jZQ==\r\nX-Injected: 1",
        ] {
            let result = validate_upgrade_request("GET", &with_header("Sec-WebSocket-Key", key));
            assert!(result.is_err(), "key {key:?} must be rejected");
            assert!(
                result.unwrap_err().contains("Sec-WebSocket-Key"),
                "rejection for {key:?} must name the key header"
            );
        }

        let nonce = BASE64.encode([0xA5_u8; 16]);
        assert_eq!(
            validate_upgrade_request("GET", &with_header("Sec-WebSocket-Key", &nonce)).unwrap(),
            nonce
        );

        // The whole request parser reports the same rejection instead of
        // producing a 101 for an impostor upgrade.
        let request = b"GET /ws HTTP/1.1\r\nUpgrade: notwebsocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";
        assert!(parse_upgrade_request_bytes(request).is_err());
        let request = b"GET /ws HTTP/1.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: short\r\nSec-WebSocket-Version: 13\r\n\r\n";
        assert!(parse_upgrade_request_bytes(request).is_err());
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
}
