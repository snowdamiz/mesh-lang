@native("gate_native_text")
pub fn native_text() -> String

@native("gate_native_text_calls")
pub fn text_calls() -> Int

@native("gate_native_invalid_utf8")
pub fn invalid_utf8() -> String

@native("gate_native_convert")
pub fn from_utf8(text :: String) -> Bytes

@native("gate_native_consume")
pub fn consume(bytes :: Bytes) -> Int

@native("mesh_bytes_from_utf8")
pub fn abi_from_utf8(text :: String) -> Bytes
