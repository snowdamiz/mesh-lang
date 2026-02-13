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

// Silence unused warning until Plan 03 uses this constant.
const _: () = { let _ = MAX_COLLECTION_LEN; };

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
