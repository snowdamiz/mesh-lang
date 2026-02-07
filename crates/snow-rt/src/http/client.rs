//! HTTP client runtime for the Snow language.
//!
//! Uses `ureq` for HTTP requests. Returns SnowResult (Ok/Err) with
//! the response body or error message.

use crate::gc::snow_gc_alloc;
use crate::io::SnowResult;
use crate::string::{snow_string_new, SnowString};

/// Allocate a SnowResult on the GC heap.
fn alloc_result(tag: u8, value: *mut u8) -> *mut SnowResult {
    unsafe {
        let ptr = snow_gc_alloc(
            std::mem::size_of::<SnowResult>() as u64,
            std::mem::align_of::<SnowResult>() as u64,
        ) as *mut SnowResult;
        (*ptr).tag = tag;
        (*ptr).value = value;
        ptr
    }
}

/// Make an HTTP GET request. Returns SnowResult:
/// - tag 0 (Ok): value = SnowString response body
/// - tag 1 (Err): value = SnowString error message
#[no_mangle]
pub extern "C" fn snow_http_get(url: *const SnowString) -> *mut u8 {
    unsafe {
        let url_str = (*url).as_str();
        match ureq::get(url_str).call() {
            Ok(response) => {
                let body = response.into_string().unwrap_or_default();
                let body_snow = snow_string_new(body.as_ptr(), body.len() as u64);
                alloc_result(0, body_snow as *mut u8) as *mut u8
            }
            Err(e) => {
                let msg = e.to_string();
                let msg_snow = snow_string_new(msg.as_ptr(), msg.len() as u64);
                alloc_result(1, msg_snow as *mut u8) as *mut u8
            }
        }
    }
}

/// Make an HTTP POST request with a body. Returns SnowResult:
/// - tag 0 (Ok): value = SnowString response body
/// - tag 1 (Err): value = SnowString error message
#[no_mangle]
pub extern "C" fn snow_http_post(url: *const SnowString, body: *const SnowString) -> *mut u8 {
    unsafe {
        let url_str = (*url).as_str();
        let body_str = (*body).as_str();
        match ureq::post(url_str)
            .set("Content-Type", "application/json")
            .send_string(body_str)
        {
            Ok(response) => {
                let resp_body = response.into_string().unwrap_or_default();
                let body_snow = snow_string_new(resp_body.as_ptr(), resp_body.len() as u64);
                alloc_result(0, body_snow as *mut u8) as *mut u8
            }
            Err(e) => {
                let msg = e.to_string();
                let msg_snow = snow_string_new(msg.as_ptr(), msg.len() as u64);
                alloc_result(1, msg_snow as *mut u8) as *mut u8
            }
        }
    }
}

// Note: HTTP client tests are not included since they require network access.
// The client is tested via E2E integration tests or manual testing.
