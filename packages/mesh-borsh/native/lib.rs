use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};

const MAX_HANDLES: usize = 4_096;
const MAX_BYTES: usize = 64 * 1024 * 1024;

#[repr(C)]
pub struct MeshResult {
    tag: u8,
    value: *mut u8,
}

#[repr(C)]
pub struct MeshBytes {
    len: u64,
}

#[repr(C)]
pub struct MeshString {
    len: u64,
}

#[repr(C)]
pub struct MeshWideNum {
    low: u64,
    high: u64,
}

unsafe extern "C" {
    fn mesh_gc_alloc_actor(size: u64, align: u64) -> *mut u8;
    fn mesh_string_new(data: *const u8, len: u64) -> *mut MeshString;
    fn mesh_bytes_copy_from(data: *const u8, len: u64) -> *mut MeshBytes;
}

struct Reader {
    bytes: Vec<u8>,
    offset: usize,
    max_collection: usize,
}

struct Writer {
    bytes: Vec<u8>,
    max_output: usize,
}

static READERS: OnceLock<Mutex<HashMap<i64, Reader>>> = OnceLock::new();
static WRITERS: OnceLock<Mutex<HashMap<i64, Writer>>> = OnceLock::new();
static NEXT_HANDLE: AtomicI64 = AtomicI64::new(1);

fn readers() -> &'static Mutex<HashMap<i64, Reader>> {
    READERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn writers() -> &'static Mutex<HashMap<i64, Writer>> {
    WRITERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn handle() -> Result<i64, String> {
    let value = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    if value <= 0 {
        Err("BORSH_HANDLE: handle space exhausted".into())
    } else {
        Ok(value)
    }
}

fn mesh_error(message: impl AsRef<str>) -> MeshResult {
    let message = message.as_ref().as_bytes();
    MeshResult {
        tag: 1,
        value: unsafe { mesh_string_new(message.as_ptr(), message.len() as u64) }.cast(),
    }
}

fn mesh_ok_ptr(value: *mut u8) -> MeshResult {
    MeshResult { tag: 0, value }
}

fn mesh_ok_zero() -> MeshResult {
    mesh_ok_int(0)
}

fn allocate<T>(value: T) -> *mut u8 {
    unsafe {
        let output = mesh_gc_alloc_actor(
            std::mem::size_of::<T>() as u64,
            std::mem::align_of::<T>().max(8) as u64,
        ) as *mut T;
        output.write(value);
        output.cast()
    }
}

fn mesh_ok_int(value: i64) -> MeshResult {
    mesh_ok_ptr(allocate(value))
}

fn mesh_ok_bool(value: bool) -> MeshResult {
    mesh_ok_ptr(allocate(value as u8))
}

fn mesh_ok_wide(value: u128) -> MeshResult {
    mesh_ok_ptr(allocate(MeshWideNum {
        low: value as u64,
        high: (value >> 64) as u64,
    }))
}

fn mesh_ok_bytes(value: &[u8]) -> MeshResult {
    mesh_ok_ptr(unsafe { mesh_bytes_copy_from(value.as_ptr(), value.len() as u64) }.cast())
}

fn mesh_ok_string(value: &str) -> MeshResult {
    mesh_ok_ptr(unsafe { mesh_string_new(value.as_ptr(), value.len() as u64) }.cast())
}

unsafe fn trailing_bytes<'a, T>(value: *const T, len: u64) -> Result<&'a [u8], String> {
    let len = usize::try_from(len).map_err(|_| "BORSH_LIMIT: input is too large".to_string())?;
    if value.is_null() {
        return Err("BORSH_INPUT: null Mesh value".into());
    }
    if len > MAX_BYTES {
        return Err(format!(
            "BORSH_LIMIT: input length {len} exceeds {MAX_BYTES}"
        ));
    }
    Ok(unsafe {
        std::slice::from_raw_parts(value.cast::<u8>().add(std::mem::size_of::<u64>()), len)
    })
}

unsafe fn bytes<'a>(value: *const MeshBytes) -> Result<&'a [u8], String> {
    let len = unsafe { value.as_ref() }
        .ok_or_else(|| "BORSH_INPUT: null Bytes".to_string())?
        .len;
    unsafe { trailing_bytes(value, len) }
}

unsafe fn string<'a>(value: *const MeshString) -> Result<&'a str, String> {
    let len = unsafe { value.as_ref() }
        .ok_or_else(|| "BORSH_INPUT: null String".to_string())?
        .len;
    std::str::from_utf8(unsafe { trailing_bytes(value, len)? })
        .map_err(|_| "BORSH_UTF8: invalid Mesh string".into())
}

unsafe fn wide(value: *const MeshWideNum) -> Result<u128, String> {
    let value =
        unsafe { value.as_ref() }.ok_or_else(|| "BORSH_INPUT: null wide integer".to_string())?;
    Ok((value.high as u128) << 64 | value.low as u128)
}

fn with_reader<T>(id: i64, f: impl FnOnce(&mut Reader) -> Result<T, String>) -> Result<T, String> {
    let mut values = readers()
        .lock()
        .map_err(|_| "BORSH_STATE: reader table poisoned".to_string())?;
    f(values
        .get_mut(&id)
        .ok_or_else(|| format!("BORSH_HANDLE: unknown reader {id}"))?)
}

fn with_writer<T>(id: i64, f: impl FnOnce(&mut Writer) -> Result<T, String>) -> Result<T, String> {
    let mut values = writers()
        .lock()
        .map_err(|_| "BORSH_STATE: writer table poisoned".to_string())?;
    f(values
        .get_mut(&id)
        .ok_or_else(|| format!("BORSH_HANDLE: unknown writer {id}"))?)
}

impl Reader {
    fn take(&mut self, length: usize) -> Result<&[u8], String> {
        let remaining = self.bytes.len().saturating_sub(self.offset);
        if length > remaining {
            return Err(format!(
                "BORSH_EOF: need {length} bytes at offset {}, only {remaining} remain",
                self.offset
            ));
        }
        let start = self.offset;
        self.offset += length;
        Ok(&self.bytes[start..self.offset])
    }

    fn length(&mut self) -> Result<usize, String> {
        let bytes: [u8; 4] = self.take(4)?.try_into().unwrap();
        let length = u32::from_le_bytes(bytes) as usize;
        if length > self.max_collection {
            Err(format!(
                "BORSH_LIMIT: collection length {length} exceeds {}",
                self.max_collection
            ))
        } else {
            Ok(length)
        }
    }
}

impl Writer {
    fn append(&mut self, value: &[u8]) -> Result<(), String> {
        let length = self
            .bytes
            .len()
            .checked_add(value.len())
            .ok_or_else(|| "BORSH_LIMIT: output length overflow".to_string())?;
        if length > self.max_output {
            return Err(format!(
                "BORSH_LIMIT: output length {length} exceeds {}",
                self.max_output
            ));
        }
        self.bytes.extend_from_slice(value);
        Ok(())
    }

    fn append_length_prefixed(&mut self, value: &[u8]) -> Result<(), String> {
        let length = u32::try_from(value.len())
            .map_err(|_| "BORSH_RANGE: collection exceeds u32".to_string())?;
        let additional = 4usize
            .checked_add(value.len())
            .ok_or_else(|| "BORSH_LIMIT: output length overflow".to_string())?;
        let output_length = self
            .bytes
            .len()
            .checked_add(additional)
            .ok_or_else(|| "BORSH_LIMIT: output length overflow".to_string())?;
        if output_length > self.max_output {
            return Err(format!(
                "BORSH_LIMIT: output length {output_length} exceeds {}",
                self.max_output
            ));
        }
        self.bytes.extend_from_slice(&length.to_le_bytes());
        self.bytes.extend_from_slice(value);
        Ok(())
    }
}

macro_rules! result {
    ($value:expr, $ok:expr) => {
        match $value {
            Ok(value) => ($ok)(value),
            Err(error) => mesh_error(error),
        }
    };
}

macro_rules! read_int {
    ($name:ident, $type:ty, $size:literal) => {
        #[no_mangle]
        pub extern "C" fn $name(reader: i64) -> MeshResult {
            result!(
                with_reader(reader, |reader| {
                    let bytes: [u8; $size] = reader.take($size)?.try_into().unwrap();
                    Ok(<$type>::from_le_bytes(bytes) as i64)
                }),
                mesh_ok_int
            )
        }
    };
}

macro_rules! write_int {
    ($name:ident, $type:ty) => {
        #[no_mangle]
        pub extern "C" fn $name(writer: i64, value: i64) -> MeshResult {
            let value = <$type>::try_from(value)
                .map_err(|_| format!("BORSH_RANGE: {value} does not fit {}", stringify!($type)))
                .and_then(|value| {
                    with_writer(writer, |writer| writer.append(&value.to_le_bytes()))
                });
            result!(value, |_| mesh_ok_zero())
        }
    };
}

read_int!(mesh_borsh_read_u8, u8, 1);
read_int!(mesh_borsh_read_i8, i8, 1);
read_int!(mesh_borsh_read_u16, u16, 2);
read_int!(mesh_borsh_read_i16, i16, 2);
read_int!(mesh_borsh_read_u32, u32, 4);
read_int!(mesh_borsh_read_i32, i32, 4);

write_int!(mesh_borsh_write_u8, u8);
write_int!(mesh_borsh_write_i8, i8);
write_int!(mesh_borsh_write_u16, u16);
write_int!(mesh_borsh_write_i16, i16);
write_int!(mesh_borsh_write_u32, u32);
write_int!(mesh_borsh_write_i32, i32);

#[no_mangle]
pub extern "C" fn mesh_borsh_reader(value: *const MeshBytes, max_collection: i64) -> MeshResult {
    let value = (|| unsafe {
        let bytes = bytes(value)?;
        let max_collection = usize::try_from(max_collection)
            .map_err(|_| "BORSH_LIMIT: max_collection must be non-negative".to_string())?;
        let mut values = readers()
            .lock()
            .map_err(|_| "BORSH_STATE: reader table poisoned".to_string())?;
        if values.len() >= MAX_HANDLES {
            return Err(format!("BORSH_LIMIT: at most {MAX_HANDLES} readers"));
        }
        let id = handle()?;
        values.insert(
            id,
            Reader {
                bytes: bytes.to_vec(),
                offset: 0,
                max_collection,
            },
        );
        Ok(id)
    })();
    result!(value, mesh_ok_int)
}

#[no_mangle]
pub extern "C" fn mesh_borsh_remaining(reader: i64) -> MeshResult {
    result!(
        with_reader(reader, |reader| Ok(
            (reader.bytes.len() - reader.offset) as i64
        )),
        mesh_ok_int
    )
}

#[no_mangle]
pub extern "C" fn mesh_borsh_finish_reader(reader: i64) -> MeshResult {
    let value = (|| {
        let mut values = readers()
            .lock()
            .map_err(|_| "BORSH_STATE: reader table poisoned".to_string())?;
        let reader = values
            .remove(&reader)
            .ok_or_else(|| format!("BORSH_HANDLE: unknown reader {reader}"))?;
        let remaining = reader.bytes.len() - reader.offset;
        if remaining == 0 {
            Ok(())
        } else {
            Err(format!("BORSH_TRAILING: {remaining} bytes remain"))
        }
    })();
    result!(value, |_| mesh_ok_zero())
}

#[no_mangle]
pub extern "C" fn mesh_borsh_close_reader(reader: i64) -> i64 {
    if let Ok(mut values) = readers().lock() {
        values.remove(&reader);
    }
    0
}

#[no_mangle]
pub extern "C" fn mesh_borsh_read_u64(reader: i64) -> MeshResult {
    result!(
        with_reader(reader, |reader| {
            Ok(u64::from_le_bytes(reader.take(8)?.try_into().unwrap()) as u128)
        }),
        mesh_ok_wide
    )
}

#[no_mangle]
pub extern "C" fn mesh_borsh_read_i64(reader: i64) -> MeshResult {
    result!(
        with_reader(reader, |reader| {
            let value = i64::from_le_bytes(reader.take(8)?.try_into().unwrap()) as i128;
            Ok(value as u128)
        }),
        mesh_ok_wide
    )
}

#[no_mangle]
pub extern "C" fn mesh_borsh_read_u128(reader: i64) -> MeshResult {
    result!(
        with_reader(reader, |reader| Ok(u128::from_le_bytes(
            reader.take(16)?.try_into().unwrap()
        ))),
        mesh_ok_wide
    )
}

#[no_mangle]
pub extern "C" fn mesh_borsh_read_i128(reader: i64) -> MeshResult {
    result!(
        with_reader(reader, |reader| {
            let value = i128::from_le_bytes(reader.take(16)?.try_into().unwrap());
            Ok(value as u128)
        }),
        mesh_ok_wide
    )
}

#[no_mangle]
pub extern "C" fn mesh_borsh_read_bool(reader: i64) -> MeshResult {
    result!(
        with_reader(reader, |reader| match reader.take(1)?[0] {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(format!("BORSH_BOOL: expected 0 or 1, got {value}")),
        }),
        mesh_ok_bool
    )
}

#[no_mangle]
pub extern "C" fn mesh_borsh_read_fixed(reader: i64, length: i64) -> MeshResult {
    let value = usize::try_from(length)
        .map_err(|_| "BORSH_LIMIT: fixed length must be non-negative".to_string())
        .and_then(|length| {
            if length > MAX_BYTES {
                Err(format!(
                    "BORSH_LIMIT: fixed length {length} exceeds {MAX_BYTES}"
                ))
            } else {
                with_reader(reader, |reader| Ok(reader.take(length)?.to_vec()))
            }
        });
    result!(value, |value: Vec<u8>| mesh_ok_bytes(&value))
}

#[no_mangle]
pub extern "C" fn mesh_borsh_read_len(reader: i64) -> MeshResult {
    result!(
        with_reader(reader, |reader| Ok(reader.length()? as i64)),
        mesh_ok_int
    )
}

#[no_mangle]
pub extern "C" fn mesh_borsh_read_vec(reader: i64) -> MeshResult {
    let value = with_reader(reader, |reader| {
        let length = reader.length()?;
        Ok(reader.take(length)?.to_vec())
    });
    result!(value, |value: Vec<u8>| mesh_ok_bytes(&value))
}

#[no_mangle]
pub extern "C" fn mesh_borsh_read_string(reader: i64) -> MeshResult {
    let value = with_reader(reader, |reader| {
        let length = reader.length()?;
        String::from_utf8(reader.take(length)?.to_vec())
            .map_err(|_| "BORSH_UTF8: invalid UTF-8 string".into())
    });
    result!(value, |value: String| mesh_ok_string(&value))
}

#[no_mangle]
pub extern "C" fn mesh_borsh_read_option_tag(reader: i64) -> MeshResult {
    mesh_borsh_read_bool(reader)
}

#[no_mangle]
pub extern "C" fn mesh_borsh_writer(max_output: i64) -> MeshResult {
    let value = (|| {
        let max_output = usize::try_from(max_output)
            .map_err(|_| "BORSH_LIMIT: max_output must be non-negative".to_string())?;
        if max_output > MAX_BYTES {
            return Err(format!(
                "BORSH_LIMIT: max_output {max_output} exceeds {MAX_BYTES}"
            ));
        }
        let mut values = writers()
            .lock()
            .map_err(|_| "BORSH_STATE: writer table poisoned".to_string())?;
        if values.len() >= MAX_HANDLES {
            return Err(format!("BORSH_LIMIT: at most {MAX_HANDLES} writers"));
        }
        let id = handle()?;
        values.insert(
            id,
            Writer {
                bytes: Vec::new(),
                max_output,
            },
        );
        Ok(id)
    })();
    result!(value, mesh_ok_int)
}

#[no_mangle]
pub extern "C" fn mesh_borsh_finish_writer(writer: i64) -> MeshResult {
    let value = writers()
        .lock()
        .map_err(|_| "BORSH_STATE: writer table poisoned".to_string())
        .and_then(|mut values| {
            values
                .remove(&writer)
                .map(|writer| writer.bytes)
                .ok_or_else(|| format!("BORSH_HANDLE: unknown writer {writer}"))
        });
    result!(value, |value: Vec<u8>| mesh_ok_bytes(&value))
}

#[no_mangle]
pub extern "C" fn mesh_borsh_close_writer(writer: i64) -> i64 {
    if let Ok(mut values) = writers().lock() {
        values.remove(&writer);
    }
    0
}

#[no_mangle]
pub extern "C" fn mesh_borsh_write_u64(writer: i64, value: *const MeshWideNum) -> MeshResult {
    let value = unsafe { wide(value) }.and_then(|value| {
        let value =
            u64::try_from(value).map_err(|_| "BORSH_RANGE: value does not fit u64".to_string())?;
        with_writer(writer, |writer| writer.append(&value.to_le_bytes()))
    });
    result!(value, |_| mesh_ok_zero())
}

#[no_mangle]
pub extern "C" fn mesh_borsh_write_i64(writer: i64, value: *const MeshWideNum) -> MeshResult {
    let value = unsafe { wide(value) }.and_then(|value| {
        let value = i128::from_le_bytes(value.to_le_bytes());
        let value =
            i64::try_from(value).map_err(|_| "BORSH_RANGE: value does not fit i64".to_string())?;
        with_writer(writer, |writer| writer.append(&value.to_le_bytes()))
    });
    result!(value, |_| mesh_ok_zero())
}

#[no_mangle]
pub extern "C" fn mesh_borsh_write_u128(writer: i64, value: *const MeshWideNum) -> MeshResult {
    let value = unsafe { wide(value) }
        .and_then(|value| with_writer(writer, |writer| writer.append(&value.to_le_bytes())));
    result!(value, |_| mesh_ok_zero())
}

#[no_mangle]
pub extern "C" fn mesh_borsh_write_i128(writer: i64, value: *const MeshWideNum) -> MeshResult {
    mesh_borsh_write_u128(writer, value)
}

#[no_mangle]
pub extern "C" fn mesh_borsh_write_bool(writer: i64, value: bool) -> MeshResult {
    result!(
        with_writer(writer, |writer| writer.append(&[value as u8])),
        |_| mesh_ok_zero()
    )
}

#[no_mangle]
pub extern "C" fn mesh_borsh_write_fixed(writer: i64, value: *const MeshBytes) -> MeshResult {
    let value = unsafe { bytes(value) }
        .and_then(|value| with_writer(writer, |writer| writer.append(value)));
    result!(value, |_| mesh_ok_zero())
}

#[no_mangle]
pub extern "C" fn mesh_borsh_write_len(writer: i64, length: i64) -> MeshResult {
    let value = u32::try_from(length)
        .map_err(|_| format!("BORSH_RANGE: {length} does not fit u32"))
        .and_then(|length| with_writer(writer, |writer| writer.append(&length.to_le_bytes())));
    result!(value, |_| mesh_ok_zero())
}

#[no_mangle]
pub extern "C" fn mesh_borsh_write_vec(writer: i64, value: *const MeshBytes) -> MeshResult {
    let value = unsafe { bytes(value) }
        .and_then(|value| with_writer(writer, |writer| writer.append_length_prefixed(value)));
    result!(value, |_| mesh_ok_zero())
}

#[no_mangle]
pub extern "C" fn mesh_borsh_write_string(writer: i64, value: *const MeshString) -> MeshResult {
    let value = unsafe { string(value) }.and_then(|value| {
        with_writer(writer, |writer| {
            writer.append_length_prefixed(value.as_bytes())
        })
    });
    result!(value, |_| mesh_ok_zero())
}

#[no_mangle]
pub extern "C" fn mesh_borsh_write_option_tag(writer: i64, present: bool) -> MeshResult {
    mesh_borsh_write_bool(writer, present)
}
