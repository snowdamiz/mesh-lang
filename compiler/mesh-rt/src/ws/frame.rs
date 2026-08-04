//! WebSocket frame codec (RFC 6455 Section 5.2-5.3).
//!
//! Provides the low-level frame parser and writer for the WebSocket wire
//! protocol. Frames are the smallest unit of WebSocket communication.
//!
//! - [`read_frame`]: Parse a single frame from a byte stream (handles masking)
//! - [`write_frame`]: Write an unmasked server frame to a byte stream
//! - [`apply_mask`]: Symmetric XOR masking per RFC 6455 Section 5.3

use std::io::{Read, Write};

/// Maximum payload size (16 MiB production limit) to prevent OOM from malicious lengths.
const MAX_PAYLOAD_SIZE: u64 = 16 * 1024 * 1024;

/// WebSocket frame opcodes per RFC 6455 Section 5.2.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WsOpcode {
    Continuation = 0x0,
    Text = 0x1,
    Binary = 0x2,
    Close = 0x8,
    Ping = 0x9,
    Pong = 0xA,
}

impl WsOpcode {
    /// Parse a 4-bit opcode value into a `WsOpcode`.
    ///
    /// Returns `Err` for reserved/unknown opcodes (RFC 6455 requires close
    /// with code 1002 for unknown opcodes; the caller decides the response).
    pub fn from_u8(byte: u8) -> Result<WsOpcode, String> {
        match byte {
            0x0 => Ok(WsOpcode::Continuation),
            0x1 => Ok(WsOpcode::Text),
            0x2 => Ok(WsOpcode::Binary),
            0x8 => Ok(WsOpcode::Close),
            0x9 => Ok(WsOpcode::Ping),
            0xA => Ok(WsOpcode::Pong),
            _ => Err(format!("unknown opcode: 0x{:X}", byte)),
        }
    }
}

/// A parsed WebSocket frame.
#[derive(Debug)]
pub struct WsFrame {
    /// FIN bit -- `true` if this is the final fragment of a message.
    pub fin: bool,
    /// The frame opcode (text, binary, close, ping, pong, continuation).
    pub opcode: WsOpcode,
    /// The unmasked payload bytes.
    pub payload: Vec<u8>,
}

/// Incremental frame decoder for nonblocking transports.
///
/// Bytes may be supplied in arbitrarily small chunks. A declared payload
/// length is validated before the decoder waits for or allocates the payload,
/// which keeps partial reads bounded even for hostile peers.
pub(crate) struct FrameDecoder {
    buffer: Vec<u8>,
    cursor: usize,
    max_payload_size: usize,
}

impl FrameDecoder {
    pub(crate) fn new(max_payload_size: usize) -> Self {
        Self {
            buffer: Vec::new(),
            cursor: 0,
            max_payload_size: max_payload_size.min(MAX_PAYLOAD_SIZE as usize),
        }
    }

    pub(crate) fn extend(&mut self, bytes: &[u8]) -> Result<(), String> {
        // A reactor read can contain the tail of one maximum-sized frame and
        // a bounded batch of following frames. The reactor additionally
        // accounts for this buffer against its aggregate read budget.
        let max_buffered = self.max_payload_size.saturating_add(64 * 1024 + 14);
        if self
            .buffered_len()
            .checked_add(bytes.len())
            .is_none_or(|len| len > max_buffered)
        {
            return Err("WebSocket incremental read buffer is full".to_string());
        }
        if self.cursor > 0
            && self
                .buffer
                .len()
                .checked_add(bytes.len())
                .is_none_or(|len| len > max_buffered)
        {
            self.compact();
        }
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    pub(crate) fn buffered_len(&self) -> usize {
        self.buffer.len() - self.cursor
    }

    pub(crate) fn next_frame(&mut self) -> Result<Option<(WsFrame, bool)>, String> {
        let buffer = &self.buffer[self.cursor..];
        if buffer.len() < 2 {
            return Ok(None);
        }

        let first = buffer[0];
        let second = buffer[1];
        let fin = first & 0x80 != 0;
        if first & 0x70 != 0 {
            return Err("non-zero RSV bits without negotiated extensions".to_string());
        }
        let opcode = WsOpcode::from_u8(first & 0x0f)?;
        let is_control = matches!(opcode, WsOpcode::Close | WsOpcode::Ping | WsOpcode::Pong);
        if is_control && !fin {
            return Err("control frames must not be fragmented".to_string());
        }

        let masked = second & 0x80 != 0;
        let mut header_len = 2usize;
        let payload_len = match second & 0x7f {
            length @ 0..=125 => length as u64,
            126 => {
                if buffer.len() < 4 {
                    return Ok(None);
                }
                header_len = 4;
                u16::from_be_bytes([buffer[2], buffer[3]]) as u64
            }
            127 => {
                if buffer.len() < 10 {
                    return Ok(None);
                }
                header_len = 10;
                let length = u64::from_be_bytes(
                    buffer[2..10]
                        .try_into()
                        .expect("fixed WebSocket length slice"),
                );
                if length >> 63 != 0 {
                    return Err("MSB of 64-bit length must be 0".to_string());
                }
                length
            }
            _ => unreachable!("WebSocket length marker is masked to seven bits"),
        };
        if payload_len > self.max_payload_size as u64 {
            return Err(format!(
                "payload length {payload_len} exceeds configured maximum {}",
                self.max_payload_size
            ));
        }
        if is_control && payload_len > 125 {
            return Err("control frame payload exceeds 125 bytes".to_string());
        }

        let mask_len = if masked { 4 } else { 0 };
        let total_len = header_len
            .checked_add(mask_len)
            .and_then(|len| len.checked_add(payload_len as usize))
            .ok_or_else(|| "WebSocket frame length overflow".to_string())?;
        if buffer.len() < total_len {
            return Ok(None);
        }

        let mask_key = if masked {
            Some(
                buffer[header_len..header_len + 4]
                    .try_into()
                    .expect("fixed WebSocket mask slice"),
            )
        } else {
            None
        };
        let payload_start = header_len + mask_len;
        let mut payload = buffer[payload_start..total_len].to_vec();
        if let Some(mask_key) = mask_key {
            apply_mask(&mut payload, &mask_key);
        }
        self.cursor += total_len;
        if self.cursor == self.buffer.len() {
            if self.buffer.capacity() > 64 * 1024 + 14 {
                self.buffer = Vec::new();
            } else {
                self.buffer.clear();
            }
            self.cursor = 0;
        } else if self.cursor >= 64 * 1024 && self.cursor >= self.buffer.len() / 2 {
            self.compact();
        }

        Ok(Some((
            WsFrame {
                fin,
                opcode,
                payload,
            },
            masked,
        )))
    }

    fn compact(&mut self) {
        let remaining = self.buffered_len();
        self.buffer.copy_within(self.cursor.., 0);
        self.buffer.truncate(remaining);
        self.cursor = 0;
    }
}

pub(crate) struct MessageAssembler {
    initial_opcode: Option<WsOpcode>,
    buffer: Vec<u8>,
    max_message_size: usize,
}

pub(crate) enum ReassembleResult {
    Complete(WsFrame),
    Accumulating,
    TooLarge,
    ProtocolError(&'static str),
}

impl MessageAssembler {
    pub(crate) fn new(max_message_size: usize) -> Self {
        Self {
            initial_opcode: None,
            buffer: Vec::new(),
            max_message_size,
        }
    }

    pub(crate) fn push(&mut self, frame: WsFrame) -> ReassembleResult {
        match frame.opcode {
            WsOpcode::Text | WsOpcode::Binary if frame.fin && self.initial_opcode.is_none() => {
                if frame.payload.len() > self.max_message_size {
                    ReassembleResult::TooLarge
                } else {
                    ReassembleResult::Complete(frame)
                }
            }
            WsOpcode::Text | WsOpcode::Binary if !frame.fin && self.initial_opcode.is_none() => {
                self.initial_opcode = Some(frame.opcode);
                self.buffer = frame.payload;
                if self.buffer.len() > self.max_message_size {
                    self.reset();
                    ReassembleResult::TooLarge
                } else {
                    ReassembleResult::Accumulating
                }
            }
            WsOpcode::Text | WsOpcode::Binary if self.initial_opcode.is_some() => {
                self.reset();
                ReassembleResult::ProtocolError("new message during fragmented sequence")
            }
            WsOpcode::Continuation if self.initial_opcode.is_some() => {
                if self
                    .buffer
                    .len()
                    .checked_add(frame.payload.len())
                    .is_none_or(|len| len > self.max_message_size)
                {
                    self.reset();
                    return ReassembleResult::TooLarge;
                }
                self.buffer.extend_from_slice(&frame.payload);
                if frame.fin {
                    let opcode = self.initial_opcode.take().unwrap();
                    ReassembleResult::Complete(WsFrame {
                        fin: true,
                        opcode,
                        payload: std::mem::take(&mut self.buffer),
                    })
                } else {
                    ReassembleResult::Accumulating
                }
            }
            WsOpcode::Continuation => {
                ReassembleResult::ProtocolError("unexpected continuation frame")
            }
            _ => ReassembleResult::ProtocolError("unexpected opcode in reassembly"),
        }
    }

    pub(crate) fn buffered_len(&self) -> usize {
        self.buffer.len()
    }

    fn reset(&mut self) {
        self.initial_opcode = None;
        self.buffer.clear();
    }
}

/// Apply or remove the 4-byte XOR mask on a payload.
///
/// The operation is symmetric: applying the mask twice returns the original.
/// Per RFC 6455 Section 5.3.
pub fn apply_mask(payload: &mut [u8], mask_key: &[u8; 4]) {
    for (i, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask_key[i % 4];
    }
}

/// Parse one WebSocket frame from the stream.
///
/// Handles all three payload length encodings (7-bit, 16-bit, 64-bit) and
/// XOR unmasking of client-to-server frames. Uses `read_exact` for all reads
/// -- the caller controls buffering.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<WsFrame, String> {
    read_frame_with_mask(reader).map(|(frame, _)| frame)
}

pub(crate) fn read_frame_with_mask<R: Read>(reader: &mut R) -> Result<(WsFrame, bool), String> {
    // Byte 0: FIN(1) RSV(3) Opcode(4)
    // Byte 1: MASK(1) Payload-Length(7)
    let mut header = [0u8; 2];
    reader
        .read_exact(&mut header)
        .map_err(|e| format!("read frame header: {}", e))?;

    let fin = (header[0] & 0x80) != 0;
    let rsv = (header[0] >> 4) & 0x07;
    if rsv != 0 {
        return Err("non-zero RSV bits without negotiated extensions".to_string());
    }
    let opcode_byte = header[0] & 0x0F;
    let opcode = WsOpcode::from_u8(opcode_byte)?;
    let is_control = matches!(opcode, WsOpcode::Close | WsOpcode::Ping | WsOpcode::Pong);
    if is_control && !fin {
        return Err("control frames must not be fragmented".to_string());
    }

    let masked = (header[1] & 0x80) != 0;
    let length_byte = header[1] & 0x7F;

    // Payload length: 3 encodings per RFC 6455 Section 5.2
    let payload_len: u64 = match length_byte {
        0..=125 => length_byte as u64,
        126 => {
            let mut buf = [0u8; 2];
            reader
                .read_exact(&mut buf)
                .map_err(|e| format!("read 16-bit length: {}", e))?;
            u16::from_be_bytes(buf) as u64
        }
        127 => {
            let mut buf = [0u8; 8];
            reader
                .read_exact(&mut buf)
                .map_err(|e| format!("read 64-bit length: {}", e))?;
            let len = u64::from_be_bytes(buf);
            if len >> 63 != 0 {
                return Err("MSB of 64-bit length must be 0".to_string());
            }
            len
        }
        _ => unreachable!(),
    };

    // Safety cap to prevent OOM from malicious lengths
    if payload_len > MAX_PAYLOAD_SIZE {
        return Err(format!(
            "payload length {} exceeds maximum {}",
            payload_len, MAX_PAYLOAD_SIZE
        ));
    }
    if is_control && payload_len > 125 {
        return Err("control frame payload exceeds 125 bytes".to_string());
    }

    // Masking key (4 bytes, present only if MASK bit is set)
    let mask_key = if masked {
        let mut key = [0u8; 4];
        reader
            .read_exact(&mut key)
            .map_err(|e| format!("read mask key: {}", e))?;
        Some(key)
    } else {
        None
    };

    // Read payload
    let mut payload = vec![0u8; payload_len as usize];
    if payload_len > 0 {
        reader
            .read_exact(&mut payload)
            .map_err(|e| format!("read payload: {}", e))?;
    }

    // Unmask if needed (client-to-server frames MUST be masked)
    if let Some(key) = mask_key {
        apply_mask(&mut payload, &key);
    }

    Ok((
        WsFrame {
            fin,
            opcode,
            payload,
        },
        masked,
    ))
}

/// Write one WebSocket frame to the stream (server-to-client, unmasked).
///
/// Server MUST NOT mask frames per RFC 6455 Section 5.1. Uses the three
/// payload length encodings depending on payload size.
pub fn write_frame<W: Write>(
    writer: &mut W,
    opcode: WsOpcode,
    payload: &[u8],
    fin: bool,
) -> Result<(), String> {
    write_frame_with_mask(writer, opcode, payload, fin, None)
}

/// Write one masked client-to-server WebSocket frame.
pub fn write_masked_frame<W: Write>(
    writer: &mut W,
    opcode: WsOpcode,
    payload: &[u8],
    fin: bool,
    mask_key: [u8; 4],
) -> Result<(), String> {
    write_frame_with_mask(writer, opcode, payload, fin, Some(mask_key))
}

fn write_frame_with_mask<W: Write>(
    writer: &mut W,
    opcode: WsOpcode,
    payload: &[u8],
    fin: bool,
    mask_key: Option<[u8; 4]>,
) -> Result<(), String> {
    let is_control = matches!(opcode, WsOpcode::Close | WsOpcode::Ping | WsOpcode::Pong);
    if is_control && (!fin || payload.len() > 125) {
        return Err("control frames must be final and at most 125 bytes".to_string());
    }

    // Byte 0: FIN + opcode
    let byte0 = if fin { 0x80 } else { 0x00 } | (opcode as u8);

    let mask_bit = if mask_key.is_some() { 0x80 } else { 0 };
    let len = payload.len();
    if len <= 125 {
        writer
            .write_all(&[byte0, mask_bit | len as u8])
            .map_err(|e| format!("write frame header: {}", e))?;
    } else if len <= 65535 {
        writer
            .write_all(&[byte0, mask_bit | 126])
            .map_err(|e| format!("write frame header: {}", e))?;
        writer
            .write_all(&(len as u16).to_be_bytes())
            .map_err(|e| format!("write 16-bit length: {}", e))?;
    } else {
        writer
            .write_all(&[byte0, mask_bit | 127])
            .map_err(|e| format!("write frame header: {}", e))?;
        writer
            .write_all(&(len as u64).to_be_bytes())
            .map_err(|e| format!("write 64-bit length: {}", e))?;
    }

    if let Some(mask_key) = mask_key {
        writer
            .write_all(&mask_key)
            .map_err(|e| format!("write mask key: {}", e))?;
        let mut masked = payload.to_vec();
        apply_mask(&mut masked, &mask_key);
        writer
            .write_all(&masked)
            .map_err(|e| format!("write payload: {}", e))?;
    } else if !payload.is_empty() {
        writer
            .write_all(payload)
            .map_err(|e| format!("write payload: {}", e))?;
    }

    writer.flush().map_err(|e| format!("flush frame: {}", e))
}

pub(crate) fn encode_frame(
    opcode: WsOpcode,
    payload: &[u8],
    fin: bool,
    mask_key: Option<[u8; 4]>,
) -> Result<Vec<u8>, String> {
    let mut encoded = Vec::with_capacity(payload.len().saturating_add(14));
    write_frame_with_mask(&mut encoded, opcode, payload, fin, mask_key)?;
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn incremental_decoder_preserves_partial_headers_and_payloads() {
        let mut encoded = Vec::new();
        write_masked_frame(
            &mut encoded,
            WsOpcode::Text,
            b"split across reads",
            true,
            [1, 2, 3, 4],
        )
        .unwrap();
        let mut decoder = FrameDecoder::new(1024);

        for byte in encoded.iter().take(encoded.len() - 1) {
            decoder.extend(&[*byte]).unwrap();
            assert!(decoder.next_frame().unwrap().is_none());
        }
        decoder.extend(&encoded[encoded.len() - 1..]).unwrap();

        let (frame, masked) = decoder.next_frame().unwrap().unwrap();
        assert!(masked);
        assert_eq!(frame.opcode, WsOpcode::Text);
        assert_eq!(frame.payload, b"split across reads");
        assert!(decoder.next_frame().unwrap().is_none());
    }

    #[test]
    fn incremental_decoder_emits_coalesced_frames_and_rejects_declared_overflow() {
        let mut encoded = Vec::new();
        write_frame(&mut encoded, WsOpcode::Text, b"one", true).unwrap();
        write_frame(&mut encoded, WsOpcode::Binary, b"two", true).unwrap();
        let mut decoder = FrameDecoder::new(8);
        decoder.extend(&encoded).unwrap();

        assert_eq!(decoder.next_frame().unwrap().unwrap().0.payload, b"one");
        assert_eq!(decoder.next_frame().unwrap().unwrap().0.payload, b"two");

        let mut oversized = FrameDecoder::new(8);
        oversized.extend(&[0x82, 126, 0, 9]).unwrap();
        assert!(oversized
            .next_frame()
            .unwrap_err()
            .contains("exceeds configured maximum"));
    }

    #[test]
    fn incremental_decoder_compacts_coalesced_small_masked_frames() {
        let frame_count = 50_000;
        let mut encoded = Vec::with_capacity(frame_count * 7);
        for payload in (0..frame_count).map(|value| value as u8) {
            encoded.extend_from_slice(&[0x82, 0x81, 0, 0, 0, 0, payload]);
        }
        let mut decoder = FrameDecoder::new(encoded.len());
        decoder.extend(&encoded).unwrap();

        for expected in (0..1_000).map(|value| value as u8) {
            assert_eq!(decoder.next_frame().unwrap().unwrap().0.payload, [expected]);
        }
        assert_eq!(decoder.cursor, 7_000);
        assert_eq!(decoder.buffer.len(), encoded.len());

        for expected in (1_000..frame_count).map(|value| value as u8) {
            assert_eq!(decoder.next_frame().unwrap().unwrap().0.payload, [expected]);
        }
        assert_eq!(decoder.buffered_len(), 0);
        assert_eq!(decoder.cursor, 0);
        assert!(decoder.buffer.is_empty());
        assert_eq!(decoder.buffer.capacity(), 0);
    }

    #[test]
    fn test_mask_roundtrip() {
        let original = b"Hello".to_vec();
        let key = [0x37, 0xfa, 0x21, 0x3d];
        let mut masked = original.clone();
        apply_mask(&mut masked, &key);
        assert_ne!(masked, original, "masked should differ from original");
        apply_mask(&mut masked, &key);
        assert_eq!(masked, original, "unmasked should equal original");
    }

    #[test]
    fn test_read_7bit_text_frame() {
        // A masked text frame "Hi" (2 bytes) from client
        // FIN=1, opcode=0x1 (text), MASK=1, len=2, mask_key=[0,0,0,0], payload="Hi"
        let frame_bytes: Vec<u8> = vec![
            0x81, // FIN=1, opcode=0x1
            0x82, // MASK=1, len=2
            0, 0, 0, 0, // mask key (all zeros = payload unchanged)
            b'H', b'i', // payload
        ];
        let mut cursor = Cursor::new(frame_bytes);
        let frame = read_frame(&mut cursor).unwrap();
        assert!(frame.fin);
        assert_eq!(frame.opcode, WsOpcode::Text);
        assert_eq!(frame.payload, b"Hi");
    }

    #[test]
    fn test_read_16bit_length() {
        // A masked frame with 200-byte payload using 16-bit length encoding
        let payload = vec![0xABu8; 200];
        let mask_key = [0u8; 4]; // zero mask for simplicity

        let mut frame_bytes: Vec<u8> = Vec::new();
        frame_bytes.push(0x82); // FIN=1, opcode=Binary
        frame_bytes.push(0xFE); // MASK=1, len=126 (16-bit follows)
        frame_bytes.extend_from_slice(&200u16.to_be_bytes()); // 16-bit length
        frame_bytes.extend_from_slice(&mask_key); // mask key
        frame_bytes.extend_from_slice(&payload); // payload

        let mut cursor = Cursor::new(frame_bytes);
        let frame = read_frame(&mut cursor).unwrap();
        assert!(frame.fin);
        assert_eq!(frame.opcode, WsOpcode::Binary);
        assert_eq!(frame.payload.len(), 200);
        assert_eq!(frame.payload, payload);
    }

    #[test]
    fn test_read_64bit_length() {
        // A masked frame with 300-byte payload using 64-bit length encoding
        let payload = vec![0xCDu8; 300];
        let mask_key = [0u8; 4]; // zero mask for simplicity

        let mut frame_bytes: Vec<u8> = Vec::new();
        frame_bytes.push(0x82); // FIN=1, opcode=Binary
        frame_bytes.push(0xFF); // MASK=1, len=127 (64-bit follows)
        frame_bytes.extend_from_slice(&300u64.to_be_bytes()); // 64-bit length
        frame_bytes.extend_from_slice(&mask_key); // mask key
        frame_bytes.extend_from_slice(&payload); // payload

        let mut cursor = Cursor::new(frame_bytes);
        let frame = read_frame(&mut cursor).unwrap();
        assert!(frame.fin);
        assert_eq!(frame.opcode, WsOpcode::Binary);
        assert_eq!(frame.payload.len(), 300);
        assert_eq!(frame.payload, payload);
    }

    #[test]
    fn test_write_small_frame() {
        // Write a text frame "Hello" (unmasked server frame)
        let mut buf = Vec::new();
        write_frame(&mut buf, WsOpcode::Text, b"Hello", true).unwrap();
        assert_eq!(buf, vec![0x81, 0x05, b'H', b'e', b'l', b'l', b'o']);
    }

    #[test]
    fn test_write_medium_frame() {
        // Write a 200-byte frame, verify 16-bit length encoding
        let payload = vec![0x42u8; 200];
        let mut buf = Vec::new();
        write_frame(&mut buf, WsOpcode::Binary, &payload, true).unwrap();

        // Header: FIN=1 + opcode=Binary(0x2) = 0x82, len=126, then 200 as u16 BE
        assert_eq!(buf[0], 0x82);
        assert_eq!(buf[1], 126);
        assert_eq!(&buf[2..4], &200u16.to_be_bytes());
        assert_eq!(&buf[4..], &payload[..]);
    }

    #[test]
    fn test_unknown_opcode() {
        // Frame with opcode 0x03 (reserved)
        let frame_bytes: Vec<u8> = vec![
            0x83, // FIN=1, opcode=0x3 (reserved)
            0x00, // MASK=0, len=0
        ];
        let mut cursor = Cursor::new(frame_bytes);
        let result = read_frame(&mut cursor);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("unknown opcode"),
            "error should mention unknown opcode, got: {}",
            err
        );
    }

    #[test]
    fn test_nonzero_rsv_rejected() {
        // Frame with RSV1 bit set: byte0 = 0xC1 (FIN=1, RSV1=1, opcode=Text)
        let frame_bytes: Vec<u8> = vec![
            0xC1, // FIN=1, RSV1=1, opcode=0x1
            0x00, // MASK=0, len=0
        ];
        let mut cursor = Cursor::new(frame_bytes);
        let result = read_frame(&mut cursor);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("RSV"),
            "error should mention RSV bits, got: {}",
            err
        );
    }

    #[test]
    fn rejects_fragmented_or_oversized_control_frames() {
        assert!(read_frame(&mut Cursor::new([0x09, 0x00])).is_err());

        let mut oversized_ping = vec![0x89, 126, 0, 126];
        oversized_ping.extend(std::iter::repeat_n(0, 126));
        assert!(read_frame(&mut Cursor::new(oversized_ping)).is_err());
    }

    #[test]
    fn test_frame_roundtrip() {
        // Write a frame then read it back (unmasked server frame)
        let original_payload = b"round-trip test payload";
        let mut buf = Vec::new();
        write_frame(&mut buf, WsOpcode::Text, original_payload, true).unwrap();

        let mut cursor = Cursor::new(buf);
        let frame = read_frame(&mut cursor).unwrap();
        assert!(frame.fin);
        assert_eq!(frame.opcode, WsOpcode::Text);
        assert_eq!(frame.payload, original_payload);
    }

    #[test]
    fn client_frame_writer_masks_payload() {
        let mut buf = Vec::new();
        write_masked_frame(
            &mut buf,
            WsOpcode::Text,
            b"client payload",
            true,
            [1, 2, 3, 4],
        )
        .unwrap();

        assert_ne!(buf[1] & 0x80, 0);
        let mut cursor = Cursor::new(buf);
        let frame = read_frame(&mut cursor).unwrap();
        assert_eq!(frame.payload, b"client payload");
    }
}
