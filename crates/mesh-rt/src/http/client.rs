//! HTTP client runtime for the Mesh language.
//!
//! Uses `ureq` for HTTP requests. Returns MeshResult (Ok/Err) with
//! the response body or error message.

use crate::gc::mesh_gc_alloc_actor;
use crate::io::MeshResult;
use crate::string::{mesh_string_new, MeshString};

/// Allocate a MeshResult on the GC heap.
fn alloc_result(tag: u8, value: *mut u8) -> *mut MeshResult {
    unsafe {
        let ptr = mesh_gc_alloc_actor(
            std::mem::size_of::<MeshResult>() as u64,
            std::mem::align_of::<MeshResult>() as u64,
        ) as *mut MeshResult;
        (*ptr).tag = tag;
        (*ptr).value = value;
        ptr
    }
}

/// Make an HTTP GET request. Returns MeshResult:
/// - tag 0 (Ok): value = MeshString response body
/// - tag 1 (Err): value = MeshString error message
#[no_mangle]
pub extern "C" fn mesh_http_get(url: *const MeshString) -> *mut u8 {
    unsafe {
        let url_str = (*url).as_str();
        match ureq::get(url_str).call() {
            Ok(response) => {
                let body = response.into_string().unwrap_or_default();
                let body_mesh = mesh_string_new(body.as_ptr(), body.len() as u64);
                alloc_result(0, body_mesh as *mut u8) as *mut u8
            }
            Err(e) => {
                let msg = e.to_string();
                let msg_mesh = mesh_string_new(msg.as_ptr(), msg.len() as u64);
                alloc_result(1, msg_mesh as *mut u8) as *mut u8
            }
        }
    }
}

/// Make an HTTP POST request with a body. Returns MeshResult:
/// - tag 0 (Ok): value = MeshString response body
/// - tag 1 (Err): value = MeshString error message
#[no_mangle]
pub extern "C" fn mesh_http_post(url: *const MeshString, body: *const MeshString) -> *mut u8 {
    unsafe {
        let url_str = (*url).as_str();
        let body_str = (*body).as_str();
        match ureq::post(url_str)
            .set("Content-Type", "application/json")
            .send_string(body_str)
        {
            Ok(response) => {
                let resp_body = response.into_string().unwrap_or_default();
                let body_mesh = mesh_string_new(resp_body.as_ptr(), resp_body.len() as u64);
                alloc_result(0, body_mesh as *mut u8) as *mut u8
            }
            Err(e) => {
                let msg = e.to_string();
                let msg_mesh = mesh_string_new(msg.as_ptr(), msg.len() as u64);
                alloc_result(1, msg_mesh as *mut u8) as *mut u8
            }
        }
    }
}

// Note: HTTP client tests are not included since they require network access.
// The client is tested via E2E integration tests or manual testing.
