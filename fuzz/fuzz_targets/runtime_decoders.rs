#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use mesh_rt::dist::protocol::{ProtocolEnvelope, ProtocolHello, MAX_NEGOTIATED_FRAME_BYTES};
use mesh_rt::dist::routing::NodeLoadReport;
use mesh_rt::ws::{parse_close_payload, read_frame, validate_text_payload};

fuzz_target!(|data: &[u8]| {
    let input = &data[..data.len().min(64 * 1_024)];
    let _ = ProtocolHello::decode(input);
    let frame_limit = input
        .get(..4)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .unwrap_or_default()
        % MAX_NEGOTIATED_FRAME_BYTES.max(1);
    let _ = ProtocolEnvelope::decode(input, frame_limit.max(64));
    let _ = NodeLoadReport::decode(input);
    let _ = read_frame(&mut Cursor::new(input));
    let _ = parse_close_payload(input);
    let _ = validate_text_payload(input);
});
