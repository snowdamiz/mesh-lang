//! WebSocket frame codec (RFC 6455 Section 5.2-5.3).
//!
//! Provides the low-level frame parser and writer for the WebSocket wire
//! protocol. Frames are the smallest unit of WebSocket communication.
//!
//! - [`FrameDecoder`]: Parse frames from bytes as a nonblocking reader gets them
//! - [`read_frame`]: Parse a single frame from a blocking byte stream
//! - [`write_frame`]: Write an unmasked server frame to a byte stream
//! - [`apply_mask`]: Symmetric XOR masking per RFC 6455 Section 5.3

use std::io::{Read, Write};

/// Maximum payload size (16 MiB production limit) to prevent OOM from malicious lengths.
const MAX_PAYLOAD_SIZE: u64 = 16 * 1024 * 1024;

/// Consumed bytes a decoder keeps before it lets them go, and spare
/// capacity it keeps then: a large frame's allocation goes with it.
const COMPACT_AFTER: usize = 64 * 1024 + 14;

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

    fn is_control(self) -> bool {
        matches!(self, WsOpcode::Close | WsOpcode::Ping | WsOpcode::Pong)
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

/// A frame's header, and the length of what follows it.
struct FrameHeader {
    fin: bool,
    opcode: WsOpcode,
    mask_key: Option<[u8; 4]>,
    /// The header's own length, its mask key included.
    header_len: usize,
    payload_len: usize,
}

impl FrameHeader {
    /// The header at the start of `bytes`, of a frame whose payload may be
    /// at most `max_payload` bytes: none until `bytes` holds all of it.
    fn parse(bytes: &[u8], max_payload: usize) -> Result<Option<Self>, String> {
        let [first, second, ..] = *bytes else {
            return Ok(None);
        };
        let fin = first & 0x80 != 0;
        if first & 0x70 != 0 {
            return Err("non-zero RSV bits without negotiated extensions".to_string());
        }
        let opcode = WsOpcode::from_u8(first & 0x0f)?;
        if opcode.is_control() && !fin {
            return Err("control frames must not be fragmented".to_string());
        }
        // A 7-bit length, or a marker for the 16- or 64-bit one after it.
        let (length_len, payload_len) = match second & 0x7f {
            126 => (
                2,
                bytes
                    .get(2..4)
                    .map(|b| u16::from_be_bytes([b[0], b[1]]) as u64),
            ),
            127 => (
                8,
                bytes
                    .get(2..10)
                    .map(|b| u64::from_be_bytes(b.try_into().expect("eight length bytes"))),
            ),
            length => (0, Some(u64::from(length))),
        };
        let Some(payload_len) = payload_len else {
            return Ok(None);
        };
        if payload_len > max_payload as u64 {
            return Err(format!(
                "payload length {payload_len} exceeds configured maximum {max_payload}"
            ));
        }
        if opcode.is_control() && payload_len > 125 {
            return Err("control frame payload exceeds 125 bytes".to_string());
        }
        let masked = second & 0x80 != 0;
        let header_len = 2 + length_len + if masked { 4 } else { 0 };
        let Some(header) = bytes.get(..header_len) else {
            return Ok(None);
        };
        Ok(Some(Self {
            fin,
            opcode,
            mask_key: masked.then(|| {
                header[header_len - 4..]
                    .try_into()
                    .expect("four mask bytes")
            }),
            header_len,
            payload_len: payload_len as usize,
        }))
    }

    /// The frame of this header and `payload`, unmasked; and whether it was
    /// masked.
    fn frame(self, mut payload: Vec<u8>) -> (WsFrame, bool) {
        if let Some(mask_key) = self.mask_key {
            apply_mask(&mut payload, &mask_key);
        }
        (
            WsFrame {
                fin: self.fin,
                opcode: self.opcode,
                payload,
            },
            self.mask_key.is_some(),
        )
    }
}

/// Incremental frame decoder for nonblocking transports.
///
/// Bytes may be supplied in arbitrarily small chunks. A declared payload
/// length is validated before the decoder waits for or allocates the payload,
/// and the reactor gives it more only as it takes frames out, which keeps
/// what it buffers bounded even for hostile peers.
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

    pub(crate) fn extend(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    pub(crate) fn buffered_len(&self) -> usize {
        self.buffer.len() - self.cursor
    }

    pub(crate) fn next_frame(&mut self) -> Result<Option<(WsFrame, bool)>, String> {
        let buffer = &self.buffer[self.cursor..];
        let Some(header) = FrameHeader::parse(buffer, self.max_payload_size)? else {
            return Ok(None);
        };
        let total_len = header.header_len + header.payload_len;
        let Some(payload) = buffer.get(header.header_len..total_len) else {
            return Ok(None);
        };
        let frame = header.frame(payload.to_vec());
        self.cursor += total_len;
        if self.cursor == self.buffer.len() || self.cursor > COMPACT_AFTER {
            self.compact();
        }
        Ok(Some(frame))
    }

    fn compact(&mut self) {
        let remaining = self.buffered_len();
        if self.buffer.capacity() - remaining > COMPACT_AFTER {
            self.buffer = self.buffer[self.cursor..].to_vec();
        } else {
            self.buffer.copy_within(self.cursor.., 0);
            self.buffer.truncate(remaining);
        }
        self.cursor = 0;
    }
}

/// Joins the data frames of a message: text or binary, then continuations.
/// It takes data frames only (control frames are the caller's), each at
/// most the message limit already, as the decoder allows no larger.
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
        let starts_message = frame.opcode != WsOpcode::Continuation;
        match (self.initial_opcode, starts_message) {
            (None, true) if frame.fin => ReassembleResult::Complete(frame),
            (None, true) => {
                self.initial_opcode = Some(frame.opcode);
                self.buffer = frame.payload;
                ReassembleResult::Accumulating
            }
            (None, false) => ReassembleResult::ProtocolError("unexpected continuation frame"),
            (Some(_), true) => {
                self.reset();
                ReassembleResult::ProtocolError("new message during fragmented sequence")
            }
            (Some(opcode), false) => {
                if self.buffer.len() + frame.payload.len() > self.max_message_size {
                    self.reset();
                    return ReassembleResult::TooLarge;
                }
                self.buffer.extend_from_slice(&frame.payload);
                if !frame.fin {
                    return ReassembleResult::Accumulating;
                }
                self.initial_opcode = None;
                ReassembleResult::Complete(WsFrame {
                    fin: true,
                    opcode,
                    payload: std::mem::take(&mut self.buffer),
                })
            }
        }
    }

    pub(crate) fn buffered_len(&self) -> usize {
        self.buffer.len()
    }

    fn reset(&mut self) {
        self.initial_opcode = None;
        self.buffer = Vec::new();
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
/// XOR unmasking of client-to-server frames, reading no byte past the
/// frame: the caller controls buffering.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<WsFrame, String> {
    read_frame_with_mask(reader).map(|(frame, _)| frame)
}

pub(crate) fn read_frame_with_mask<R: Read>(reader: &mut R) -> Result<(WsFrame, bool), String> {
    // The header, a byte at a time: its first bytes say how long it is.
    let mut header = Vec::with_capacity(14);
    let header = loop {
        if let Some(header) = FrameHeader::parse(&header, MAX_PAYLOAD_SIZE as usize)? {
            break header;
        }
        let mut byte = [0u8];
        reader
            .read_exact(&mut byte)
            .map_err(|e| format!("read frame header: {}", e))?;
        header.push(byte[0]);
    };
    let mut payload = vec![0u8; header.payload_len];
    reader
        .read_exact(&mut payload)
        .map_err(|e| format!("read payload: {}", e))?;
    Ok(header.frame(payload))
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
    let frame = frame_bytes(opcode, payload, fin, mask_key)?;
    writer
        .write_all(&frame)
        .and_then(|()| writer.flush())
        .map_err(|e| format!("write frame: {}", e))
}

/// A frame of `payload`, masked with `mask_key` when given. A control
/// frame must be final and at most 125 bytes.
fn frame_bytes(
    opcode: WsOpcode,
    payload: &[u8],
    fin: bool,
    mask_key: Option<[u8; 4]>,
) -> Result<Vec<u8>, String> {
    if opcode.is_control() && (!fin || payload.len() > 125) {
        return Err("control frames must be final and at most 125 bytes".to_string());
    }
    let mut frame = Vec::with_capacity(payload.len() + 14);
    frame.push(if fin { 0x80 } else { 0x00 } | opcode as u8);
    let mask_bit = if mask_key.is_some() { 0x80 } else { 0 };
    match payload.len() {
        len @ 0..=125 => frame.push(mask_bit | len as u8),
        len @ 126..=65535 => {
            frame.push(mask_bit | 126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        }
        len => {
            frame.push(mask_bit | 127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
    }
    if let Some(mask_key) = mask_key {
        frame.extend_from_slice(&mask_key);
    }
    let payload_start = frame.len();
    frame.extend_from_slice(payload);
    if let Some(mask_key) = mask_key {
        apply_mask(&mut frame[payload_start..], &mask_key);
    }
    Ok(frame)
}

/// A final frame of `payload`, masked with `mask_key` when given. The
/// reactor encodes control frames only of payloads a decoded frame or
/// `build_close_payload` bounds to 125 bytes, so encoding cannot fail.
pub(crate) fn encode_frame(opcode: WsOpcode, payload: &[u8], mask_key: Option<[u8; 4]>) -> Vec<u8> {
    frame_bytes(opcode, payload, true, mask_key)
        .expect("a final frame of a bounded control payload encodes")
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
            decoder.extend(&[*byte]);
            assert!(decoder.next_frame().unwrap().is_none());
        }
        decoder.extend(&encoded[encoded.len() - 1..]);

        let (frame, masked) = decoder.next_frame().unwrap().unwrap();
        assert!(masked);
        assert_eq!(frame.opcode, WsOpcode::Text);
        assert_eq!(frame.payload, b"split across reads");
        assert!(decoder.next_frame().unwrap().is_none());
    }

    #[test]
    fn reassembly_reset_releases_a_large_fragment_allocation() {
        let mut assembler = MessageAssembler::new(1024 * 1024);
        assert!(matches!(
            assembler.push(WsFrame {
                fin: false,
                opcode: WsOpcode::Binary,
                payload: vec![0; 256 * 1024],
            }),
            ReassembleResult::Accumulating
        ));
        assert!(assembler.buffer.capacity() >= 256 * 1024);

        assert!(matches!(
            assembler.push(WsFrame {
                fin: true,
                opcode: WsOpcode::Text,
                payload: Vec::new(),
            }),
            ReassembleResult::ProtocolError(_)
        ));
        assert_eq!(assembler.buffered_len(), 0);
        assert_eq!(assembler.buffer.capacity(), 0);
    }

    #[test]
    fn decoder_compaction_releases_consumed_frame_capacity() {
        let payload = vec![0; 256 * 1024];
        let mut encoded = Vec::new();
        write_frame(&mut encoded, WsOpcode::Binary, &payload, true).unwrap();
        encoded.push(0x82);

        let mut decoder = FrameDecoder::new(512 * 1024);
        decoder.extend(&encoded);
        assert_eq!(decoder.next_frame().unwrap().unwrap().0.payload, payload);
        assert_eq!(decoder.buffered_len(), 1);
        assert!(decoder.buffer.capacity() <= COMPACT_AFTER);
    }

    #[test]
    fn incremental_decoder_emits_coalesced_frames_and_rejects_declared_overflow() {
        let mut encoded = Vec::new();
        write_frame(&mut encoded, WsOpcode::Text, b"one", true).unwrap();
        write_frame(&mut encoded, WsOpcode::Binary, b"two", true).unwrap();
        let mut decoder = FrameDecoder::new(8);
        decoder.extend(&encoded);

        assert_eq!(decoder.next_frame().unwrap().unwrap().0.payload, b"one");
        assert_eq!(decoder.next_frame().unwrap().unwrap().0.payload, b"two");

        let mut oversized = FrameDecoder::new(8);
        oversized.extend(&[0x82, 126, 0, 9]);
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
        decoder.extend(&encoded);

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
        assert!(decoder.buffer.capacity() <= COMPACT_AFTER);
    }

    #[test]
    fn a_message_joins_any_number_of_fragments() {
        let mut assembler = MessageAssembler::new(8);
        let fragment = |opcode, payload: &[u8], fin| WsFrame {
            fin,
            opcode,
            payload: payload.to_vec(),
        };
        assert!(matches!(
            assembler.push(fragment(WsOpcode::Text, b"a", false)),
            ReassembleResult::Accumulating
        ));
        assert!(matches!(
            assembler.push(fragment(WsOpcode::Continuation, b"b", false)),
            ReassembleResult::Accumulating
        ));
        assert!(matches!(
            assembler.push(fragment(WsOpcode::Continuation, b"c", true)),
            ReassembleResult::Complete(frame)
                if frame.payload == b"abc" && frame.opcode == WsOpcode::Text
        ));
    }

    #[test]
    fn a_control_frame_is_written_final_and_short_or_not_at_all() {
        assert!(write_frame(&mut Vec::new(), WsOpcode::Ping, &[0; 126], true).is_err());
        assert!(write_frame(&mut Vec::new(), WsOpcode::Close, &[], false).is_err());
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
