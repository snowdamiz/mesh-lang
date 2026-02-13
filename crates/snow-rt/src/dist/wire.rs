//! Snow Term Format (STF) binary serializer/deserializer.
//!
//! STF is a self-describing binary format for encoding Snow runtime values
//! for inter-node message transport. Each value is prefixed by a 1-byte
//! type tag, enabling recursive serialization/deserialization.
//!
//! ## Wire Layout
//!
//! Every STF payload starts with a version byte (`STF_VERSION`), followed
//! by a single encoded value (which may recursively contain nested values).
//!
//! ## Safety Invariant
//!
//! STF encode runs as a pure Rust function operating on raw pointers -- it
//! does NOT call any Snow runtime functions that trigger `reduction_check`.
//! This means GC cannot trigger during serialization, so GC-managed objects
//! referenced by raw pointers remain valid throughout the encode operation.

// ── STF Version ──────────────────────────────────────────────────────────

/// Version byte written as the first byte of every STF payload.
pub const STF_VERSION: u8 = 1;

// ── Type Tag Constants ───────────────────────────────────────────────────

// Scalar types
pub const TAG_INT: u8 = 1; // i64, 8 bytes LE
pub const TAG_FLOAT: u8 = 2; // f64, 8 bytes LE (IEEE 754)
pub const TAG_BOOL_TRUE: u8 = 3; // no payload
pub const TAG_BOOL_FALSE: u8 = 4; // no payload
pub const TAG_STRING: u8 = 5; // u32 len + UTF-8 bytes
pub const TAG_UNIT: u8 = 6; // no payload

// Container types
pub const TAG_LIST: u8 = 10; // u32 count + count * encoded elements
pub const TAG_MAP: u8 = 11; // u8 key_type + u32 count + count * (key, value)
pub const TAG_SET: u8 = 12; // u32 count + count * encoded elements
pub const TAG_TUPLE: u8 = 13; // u8 arity + arity * encoded elements

// Composite types
pub const TAG_STRUCT: u8 = 20; // u16 name_len + name + u16 field_count + fields
pub const TAG_SUM_TYPE: u8 = 21; // u16 type_name_len + name + u8 variant_tag + u16 field_count + fields

// Identity types
pub const TAG_PID: u8 = 30; // u64 raw PID (includes node_id + creation + local_id)

// Option/Result (special-cased sum types for efficiency)
pub const TAG_OPTION_SOME: u8 = 40; // + encoded inner value
pub const TAG_OPTION_NONE: u8 = 41; // no payload
pub const TAG_RESULT_OK: u8 = 42; // + encoded inner value
pub const TAG_RESULT_ERR: u8 = 43; // + encoded inner value

// Error sentinel
pub const TAG_CLOSURE: u8 = 0xFF; // NEVER written -- triggers runtime error

// ── Safety Limits ────────────────────────────────────────────────────────

/// Maximum string length in bytes (16 MB).
const MAX_STRING_LEN: u32 = 16 * 1024 * 1024;

/// Maximum collection element count (1 million).
const MAX_COLLECTION_LEN: u32 = 1_000_000;

// Suppress unused warning until Plan 03 uses this constant.
#[allow(dead_code)]
const _MAX_COLLECTION_LEN_USED: u32 = MAX_COLLECTION_LEN;

// ── StfType ──────────────────────────────────────────────────────────────

/// Type hint enum that mirrors Snow's runtime type system.
///
/// The STF encoder requires type hints because Snow stores all values as
/// uniform `u64` at runtime (type erasure). The codegen layer provides
/// these hints when emitting remote send calls.
#[derive(Debug, Clone, PartialEq)]
pub enum StfType {
    Int,
    Float,
    Bool,
    String,
    Unit,
    Pid,
    List(Box<StfType>),                                                   // element type
    Map(Box<StfType>, Box<StfType>),                                      // key type, value type
    Set(Box<StfType>),                                                    // element type
    Tuple(Vec<StfType>),                                                  // element types
    Struct(std::string::String, Vec<(std::string::String, StfType)>),     // name, fields
    SumType(std::string::String, Vec<(std::string::String, Vec<StfType>)>), // name, variants
    OptionOf(Box<StfType>),                                               // inner type
    ResultOf(Box<StfType>, Box<StfType>),                                 // ok type, err type
    Closure,                                                              // always errors
    FnPtr,                                                                // always errors
}

// ── StfError ─────────────────────────────────────────────────────────────

/// Errors that can occur during STF encode/decode.
#[derive(Debug, Clone, PartialEq)]
pub enum StfError {
    /// The input buffer was truncated or too short.
    UnexpectedEof,
    /// An unknown or unsupported type tag was encountered.
    InvalidTag(u8),
    /// The version byte does not match `STF_VERSION`.
    InvalidVersion(u8),
    /// Attempted to serialize a closure or function pointer.
    ClosureNotSerializable,
    /// A length field exceeds the safety limit.
    PayloadTooLarge(u32),
    /// A string payload contains invalid UTF-8.
    InvalidUtf8,
}

impl std::fmt::Display for StfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StfError::UnexpectedEof => write!(f, "STF: unexpected end of input"),
            StfError::InvalidTag(tag) => write!(f, "STF: invalid type tag 0x{:02X}", tag),
            StfError::InvalidVersion(v) => write!(f, "STF: unsupported version {}", v),
            StfError::ClosureNotSerializable => {
                write!(f, "STF: closures and function pointers cannot be serialized")
            }
            StfError::PayloadTooLarge(len) => {
                write!(f, "STF: payload length {} exceeds safety limit", len)
            }
            StfError::InvalidUtf8 => write!(f, "STF: string payload is not valid UTF-8"),
        }
    }
}

// ── Encode ───────────────────────────────────────────────────────────────

use crate::string::{SnowString, snow_string_new};

/// Encode a single Snow value into the buffer (without version byte).
///
/// `value` is the raw `u64` representation of the Snow value. The
/// `type_hint` tells the encoder how to interpret the bits.
///
/// # Safety
///
/// For `StfType::String`, `value` must be a valid pointer to a `SnowString`.
pub fn stf_encode(value: u64, type_hint: &StfType, buf: &mut Vec<u8>) -> Result<(), StfError> {
    match type_hint {
        StfType::Int => {
            buf.push(TAG_INT);
            buf.extend_from_slice(&(value as i64).to_le_bytes());
            Ok(())
        }
        StfType::Float => {
            buf.push(TAG_FLOAT);
            buf.extend_from_slice(&f64::from_bits(value).to_le_bytes());
            Ok(())
        }
        StfType::Bool => {
            if value != 0 {
                buf.push(TAG_BOOL_TRUE);
            } else {
                buf.push(TAG_BOOL_FALSE);
            }
            Ok(())
        }
        StfType::String => {
            buf.push(TAG_STRING);
            let snow_str = unsafe { &*(value as *const SnowString) };
            let bytes = unsafe { snow_str.as_bytes() };
            let len = bytes.len() as u32;
            if len > MAX_STRING_LEN {
                return Err(StfError::PayloadTooLarge(len));
            }
            buf.extend_from_slice(&len.to_le_bytes());
            buf.extend_from_slice(bytes);
            Ok(())
        }
        StfType::Unit => {
            buf.push(TAG_UNIT);
            Ok(())
        }
        StfType::Pid => {
            buf.push(TAG_PID);
            buf.extend_from_slice(&value.to_le_bytes());
            Ok(())
        }
        StfType::Closure | StfType::FnPtr => Err(StfError::ClosureNotSerializable),
        // Container and composite types -- implemented in Plan 03
        StfType::List(_)
        | StfType::Map(_, _)
        | StfType::Set(_)
        | StfType::Tuple(_)
        | StfType::Struct(_, _)
        | StfType::SumType(_, _)
        | StfType::OptionOf(_)
        | StfType::ResultOf(_, _) => Err(StfError::InvalidTag(0)),
    }
}

/// Encode a Snow value with the STF version header.
///
/// Returns a complete STF payload: `[version_byte][encoded_value]`.
///
/// # Safety
///
/// Same safety requirements as [`stf_encode`].
pub fn stf_encode_value(value: u64, type_hint: &StfType) -> Result<Vec<u8>, StfError> {
    let mut buf = Vec::with_capacity(64);
    buf.push(STF_VERSION);
    stf_encode(value, type_hint, &mut buf)?;
    Ok(buf)
}

// ── Decode ───────────────────────────────────────────────────────────────

/// Helper: read exactly `n` bytes from `data` at `*pos`, advancing `*pos`.
#[inline]
fn read_bytes<'a>(data: &'a [u8], pos: &mut usize, n: usize) -> Result<&'a [u8], StfError> {
    if *pos + n > data.len() {
        return Err(StfError::UnexpectedEof);
    }
    let slice = &data[*pos..*pos + n];
    *pos += n;
    Ok(slice)
}

/// Helper: read a single byte from `data` at `*pos`, advancing `*pos`.
#[inline]
fn read_u8(data: &[u8], pos: &mut usize) -> Result<u8, StfError> {
    if *pos >= data.len() {
        return Err(StfError::UnexpectedEof);
    }
    let b = data[*pos];
    *pos += 1;
    Ok(b)
}

/// Decode a single STF value from `data` starting at `*pos`.
///
/// Returns `(raw_u64_value, decoded_type)` and advances `*pos` past
/// the consumed bytes.
///
/// # Safety
///
/// For `TAG_STRING`, this allocates a new `SnowString` via the GC
/// allocator. The caller must ensure the GC arena is initialized.
pub fn stf_decode(data: &[u8], pos: &mut usize) -> Result<(u64, StfType), StfError> {
    let tag = read_u8(data, pos)?;
    match tag {
        TAG_INT => {
            let bytes = read_bytes(data, pos, 8)?;
            let val = i64::from_le_bytes(bytes.try_into().unwrap());
            Ok((val as u64, StfType::Int))
        }
        TAG_FLOAT => {
            let bytes = read_bytes(data, pos, 8)?;
            let bits = u64::from_le_bytes(bytes.try_into().unwrap());
            Ok((bits, StfType::Float))
        }
        TAG_BOOL_TRUE => Ok((1, StfType::Bool)),
        TAG_BOOL_FALSE => Ok((0, StfType::Bool)),
        TAG_STRING => {
            let len_bytes = read_bytes(data, pos, 4)?;
            let len = u32::from_le_bytes(len_bytes.try_into().unwrap());
            if len > MAX_STRING_LEN {
                return Err(StfError::PayloadTooLarge(len));
            }
            let str_bytes = read_bytes(data, pos, len as usize)?;
            // Validate UTF-8 before allocating.
            if std::str::from_utf8(str_bytes).is_err() {
                return Err(StfError::InvalidUtf8);
            }
            let str_ptr = snow_string_new(str_bytes.as_ptr(), len as u64);
            Ok((str_ptr as u64, StfType::String))
        }
        TAG_UNIT => Ok((0, StfType::Unit)),
        TAG_PID => {
            let bytes = read_bytes(data, pos, 8)?;
            let raw_pid = u64::from_le_bytes(bytes.try_into().unwrap());
            Ok((raw_pid, StfType::Pid))
        }
        TAG_CLOSURE => Err(StfError::ClosureNotSerializable),
        // Container tags -- implemented in Plan 03
        TAG_LIST | TAG_MAP | TAG_SET | TAG_TUPLE | TAG_STRUCT | TAG_SUM_TYPE
        | TAG_OPTION_SOME | TAG_OPTION_NONE | TAG_RESULT_OK | TAG_RESULT_ERR => {
            Err(StfError::InvalidTag(tag))
        }
        unknown => Err(StfError::InvalidTag(unknown)),
    }
}

/// Decode a complete STF payload (with version header).
///
/// Validates the version byte, then decodes the value.
///
/// # Safety
///
/// Same safety requirements as [`stf_decode`].
pub fn stf_decode_value(data: &[u8]) -> Result<(u64, StfType), StfError> {
    if data.is_empty() {
        return Err(StfError::UnexpectedEof);
    }
    if data[0] != STF_VERSION {
        return Err(StfError::InvalidVersion(data[0]));
    }
    let mut pos = 1;
    stf_decode(data, &mut pos)
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::snow_rt_init;

    #[test]
    fn test_int_roundtrip() {
        snow_rt_init();
        let values: &[i64] = &[-1, 0, i64::MAX];
        for &v in values {
            let encoded = stf_encode_value(v as u64, &StfType::Int).unwrap();
            assert_eq!(encoded[0], STF_VERSION, "version byte");
            let (decoded, typ) = stf_decode_value(&encoded).unwrap();
            assert_eq!(typ, StfType::Int);
            assert_eq!(decoded as i64, v, "round-trip failed for {}", v);
        }
    }

    #[test]
    fn test_float_roundtrip() {
        snow_rt_init();
        let values: &[f64] = &[3.14, -0.0, f64::INFINITY, f64::NAN];
        for &v in values {
            let bits = v.to_bits();
            let encoded = stf_encode_value(bits, &StfType::Float).unwrap();
            assert_eq!(encoded[0], STF_VERSION, "version byte");
            let (decoded, typ) = stf_decode_value(&encoded).unwrap();
            assert_eq!(typ, StfType::Float);
            // Compare bits, not f64 values (NaN != NaN).
            assert_eq!(decoded, bits, "round-trip failed for {} (bits)", v);
        }
    }

    #[test]
    fn test_bool_roundtrip() {
        snow_rt_init();
        // true
        let encoded = stf_encode_value(1, &StfType::Bool).unwrap();
        let (val, typ) = stf_decode_value(&encoded).unwrap();
        assert_eq!(typ, StfType::Bool);
        assert_eq!(val, 1);

        // false
        let encoded = stf_encode_value(0, &StfType::Bool).unwrap();
        let (val, typ) = stf_decode_value(&encoded).unwrap();
        assert_eq!(typ, StfType::Bool);
        assert_eq!(val, 0);
    }

    #[test]
    fn test_string_roundtrip() {
        snow_rt_init();
        let test_str = "hello";
        let snow_str = snow_string_new(test_str.as_ptr(), test_str.len() as u64);
        let encoded = stf_encode_value(snow_str as u64, &StfType::String).unwrap();
        assert_eq!(encoded[0], STF_VERSION, "version byte");
        let (decoded_ptr, typ) = stf_decode_value(&encoded).unwrap();
        assert_eq!(typ, StfType::String);
        unsafe {
            let decoded_str = &*(decoded_ptr as *const SnowString);
            assert_eq!(decoded_str.as_str(), "hello");
        }
    }

    #[test]
    fn test_unit_roundtrip() {
        snow_rt_init();
        let encoded = stf_encode_value(0, &StfType::Unit).unwrap();
        assert_eq!(encoded[0], STF_VERSION, "version byte");
        let (val, typ) = stf_decode_value(&encoded).unwrap();
        assert_eq!(typ, StfType::Unit);
        assert_eq!(val, 0);
    }

    #[test]
    fn test_pid_roundtrip() {
        snow_rt_init();
        // A PID with node_id=5, creation=2, local_id=42
        let raw_pid: u64 = (5u64 << 48) | (2u64 << 40) | 42;
        let encoded = stf_encode_value(raw_pid, &StfType::Pid).unwrap();
        assert_eq!(encoded[0], STF_VERSION, "version byte");
        let (decoded, typ) = stf_decode_value(&encoded).unwrap();
        assert_eq!(typ, StfType::Pid);
        assert_eq!(decoded, raw_pid, "PID round-trip mismatch");
    }

    #[test]
    fn test_closure_rejected() {
        let result = stf_encode_value(0, &StfType::Closure);
        assert_eq!(result, Err(StfError::ClosureNotSerializable));
    }

    #[test]
    fn test_fnptr_rejected() {
        let result = stf_encode_value(0, &StfType::FnPtr);
        assert_eq!(result, Err(StfError::ClosureNotSerializable));
    }

    #[test]
    fn test_truncated_int_decode() {
        // Version byte + TAG_INT + only 4 bytes of payload (need 8).
        let mut buf = vec![STF_VERSION, TAG_INT, 0, 0, 0, 0];
        let result = stf_decode_value(&buf);
        assert_eq!(result, Err(StfError::UnexpectedEof));

        // Also test completely empty after tag.
        buf = vec![STF_VERSION, TAG_INT];
        let result = stf_decode_value(&buf);
        assert_eq!(result, Err(StfError::UnexpectedEof));
    }

    #[test]
    fn test_version_check() {
        // Wrong version byte.
        let buf = vec![99, TAG_INT, 0, 0, 0, 0, 0, 0, 0, 0];
        let result = stf_decode_value(&buf);
        assert_eq!(result, Err(StfError::InvalidVersion(99)));

        // Empty buffer.
        let result = stf_decode_value(&[]);
        assert_eq!(result, Err(StfError::UnexpectedEof));
    }
}
