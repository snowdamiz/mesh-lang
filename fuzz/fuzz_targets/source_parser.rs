#![no_main]

use libfuzzer_sys::fuzz_target;
use mesh_lexer::Lexer;
use mesh_parser::{parse, parse_block, parse_expr};

fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(&data[..data.len().min(128 * 1_024)]) else {
        return;
    };
    let _ = Lexer::tokenize(source);
    let _ = parse(source).syntax();
    let _ = parse_expr(source).syntax();
    let _ = parse_block(source).syntax();
});
