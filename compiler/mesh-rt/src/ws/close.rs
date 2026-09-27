//! WebSocket close handshake and frame validation (RFC 6455 Section 5.5.1, 7).
//!
//! Provides close frame parsing/building and text frame UTF-8 validation;
//! the reactor handles frames at the protocol level.
//!
//! - [`parse_close_payload`]: Extract status code + reason from close frame payload
//! - [`parse_close_payload_strict`]: Validate an inbound close frame payload
//! - [`build_close_payload`]: Build a close frame payload from code + reason
//! - [`is_valid_text_payload`]: UTF-8 validation for text frames (PROTO-05)

/// Well-known WebSocket close status codes per RFC 6455 Section 7.4.1.
pub struct WsCloseCode;

impl WsCloseCode {
    /// Normal closure (1000).
    pub const NORMAL: u16 = 1000;
    /// Going away (1001).
    pub const GOING_AWAY: u16 = 1001;
    /// Protocol error (1002) -- used for unknown opcodes (PROTO-09).
    pub const PROTOCOL_ERROR: u16 = 1002;
    /// Invalid frame payload data (1007) -- used for UTF-8 failure (PROTO-05).
    pub const INVALID_DATA: u16 = 1007;
    /// Message too big (1009) -- used when fragmented message exceeds 16 MiB (FRAG-03).
    pub const MESSAGE_TOO_BIG: u16 = 1009;
    /// Internal server error (1011) -- used when an actor crashes (Phase 60).
    pub const INTERNAL_ERROR: u16 = 1011;
    /// Server is overloaded and the client should reconnect later (1013).
    pub const TRY_AGAIN_LATER: u16 = 1013;
}

/// Parse a close frame payload into (status_code, reason).
///
/// Per RFC 6455 Section 7.4.1:
/// - If payload >= 2 bytes: status code is the first 2 bytes (big-endian),
///   reason is the remaining bytes decoded as UTF-8 (lossy).
/// - If payload < 2 bytes: returns (1005, "") -- 1005 means "no status code present".
pub fn parse_close_payload(payload: &[u8]) -> (u16, String) {
    if payload.len() >= 2 {
        let code = u16::from_be_bytes([payload[0], payload[1]]);
        let reason = String::from_utf8_lossy(&payload[2..]).into_owned();
        (code, reason)
    } else {
        (1005, String::new())
    }
}

/// Whether a close code may appear on the wire.
pub(crate) fn is_valid_close_code(code: u16) -> bool {
    matches!(code, 1000..=1014 | 3000..=4999) && !matches!(code, 1004..=1006)
}

/// Parse and validate an inbound close frame payload per RFC 6455 Section 7.4.
pub(crate) fn parse_close_payload_strict(payload: &[u8]) -> Result<(u16, String), String> {
    match payload {
        [] => Ok((1005, String::new())),
        [_] => Err("close payload cannot contain exactly one byte".to_string()),
        [high, low, reason @ ..] => {
            let code = u16::from_be_bytes([*high, *low]);
            if !is_valid_close_code(code) {
                return Err(format!("invalid WebSocket close code {code}"));
            }
            let reason = std::str::from_utf8(reason)
                .map_err(|_| "invalid UTF-8 in WebSocket close reason".to_string())?;
            Ok((code, reason.to_string()))
        }
    }
}

/// Build a close frame payload from a status code and reason string.
///
/// The payload is 2 bytes for the code (big-endian) followed by the reason
/// bytes. The reason is truncated to 123 bytes max so the total payload
/// stays within the 125-byte control frame limit (RFC 6455 Section 5.5).
pub fn build_close_payload(code: u16, reason: &str) -> Vec<u8> {
    let reason_bytes = reason.as_bytes();
    let max_reason_len = 123; // 125 - 2 bytes for code
    let mut truncated_len = reason_bytes.len().min(max_reason_len);
    while !reason.is_char_boundary(truncated_len) {
        truncated_len -= 1;
    }

    let mut payload = Vec::with_capacity(2 + truncated_len);
    payload.extend_from_slice(&code.to_be_bytes());
    payload.extend_from_slice(&reason_bytes[..truncated_len]);
    payload
}

/// Whether a text frame payload is valid UTF-8.
///
/// Per RFC 6455 Section 5.6, text frames MUST contain valid UTF-8.
/// Invalid UTF-8 triggers close code 1007 (PROTO-05).
pub fn is_valid_text_payload(payload: &[u8]) -> bool {
    std::str::from_utf8(payload).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_close_normal() {
        // payload [0x03, 0xE8, b'o', b'k'] -> (1000, "ok")
        let payload = vec![0x03, 0xE8, b'o', b'k'];
        let (code, reason) = parse_close_payload(&payload);
        assert_eq!(code, 1000);
        assert_eq!(reason, "ok");
    }

    #[test]
    fn test_parse_close_empty() {
        // empty payload -> (1005, "")
        let (code, reason) = parse_close_payload(&[]);
        assert_eq!(code, 1005);
        assert_eq!(reason, "");
    }

    #[test]
    fn strict_parse_accepts_empty_payload() {
        assert_eq!(parse_close_payload_strict(&[]), Ok((1005, String::new())));
    }

    #[test]
    fn strict_parse_rejects_one_byte_payload() {
        assert!(parse_close_payload_strict(&[0x03]).is_err());
    }

    #[test]
    fn strict_parse_accepts_code_and_reason() {
        assert_eq!(
            parse_close_payload_strict(&[0x03, 0xe8, b'o', b'k']),
            Ok((1000, "ok".to_string()))
        );
    }

    #[test]
    fn strict_parse_rejects_invalid_or_reserved_codes() {
        for code in [999u16, 1004, 1005, 1006, 1015, 1016, 2999, 5000] {
            assert!(
                parse_close_payload_strict(&code.to_be_bytes()).is_err(),
                "accepted invalid close code {code}"
            );
        }
    }

    #[test]
    fn strict_parse_rejects_invalid_utf8_reason() {
        let error = parse_close_payload_strict(&[0x03, 0xe8, 0xff]).unwrap_err();
        assert!(error.contains("UTF-8"));
    }

    #[test]
    fn test_parse_close_code_only() {
        // payload [0x03, 0xE8] -> (1000, "")
        let payload = vec![0x03, 0xE8];
        let (code, reason) = parse_close_payload(&payload);
        assert_eq!(code, 1000);
        assert_eq!(reason, "");
    }

    #[test]
    fn test_build_close_payload() {
        let payload = build_close_payload(1000, "bye");
        assert_eq!(payload, vec![0x03, 0xE8, b'b', b'y', b'e']);
    }

    #[test]
    fn test_build_close_truncates_reason() {
        let long_reason = "x".repeat(200);
        let payload = build_close_payload(1000, &long_reason);
        assert_eq!(
            payload.len(),
            125,
            "payload should be capped at 125 bytes (2 + 123)"
        );
        assert_eq!(&payload[..2], &[0x03, 0xE8]);
    }

    #[test]
    fn test_build_close_truncates_at_utf8_boundary() {
        let payload = build_close_payload(1000, &"🦀".repeat(40));
        assert!(payload.len() <= 125);
        assert!(std::str::from_utf8(&payload[2..]).is_ok());
    }

    #[test]
    fn test_validate_text_valid_utf8() {
        assert!(is_valid_text_payload(b"Hello"));
    }

    #[test]
    fn test_validate_text_invalid_utf8() {
        assert!(!is_valid_text_payload(&[0xFF, 0xFE]));
    }
}
