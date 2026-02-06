//! Runtime panic handler for Snow programs.
//!
//! Called when a Snow program encounters an unrecoverable error at runtime,
//! such as a non-exhaustive match failure (guarded arms edge case).

/// Panic with a source-located error message.
///
/// Prints `"Snow panic at {file}:{line}: {msg}"` to stderr and aborts
/// the process.
///
/// # Safety
///
/// `msg` must point to `msg_len` valid UTF-8 bytes.
/// `file` must point to `file_len` valid UTF-8 bytes.
#[no_mangle]
pub extern "C" fn snow_panic(
    msg: *const u8,
    msg_len: u64,
    file: *const u8,
    file_len: u64,
    line: u32,
) -> ! {
    unsafe {
        let msg = std::str::from_utf8_unchecked(std::slice::from_raw_parts(msg, msg_len as usize));
        let file =
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(file, file_len as usize));
        eprintln!("Snow panic at {}:{}: {}", file, line, msg);
        std::process::abort();
    }
}

#[cfg(test)]
mod tests {
    // Note: snow_panic aborts the process, so we cannot meaningfully test it
    // in a unit test without spawning a subprocess. The function is simple
    // enough (format + abort) that visual inspection suffices. Integration
    // tests in later phases will verify it via compiled Snow programs.
}
