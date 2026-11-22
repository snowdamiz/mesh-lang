use mesh_rt::io::{mesh_alloc_result, MeshResult};
use mesh_rt::jobs_testing;
use mesh_rt::{mesh_string_new, MeshString};

fn string(value: &str) -> *mut u8 {
    mesh_string_new(value.as_ptr(), value.len() as u64).cast()
}

fn result<T>(value: Result<T, String>, encode: impl FnOnce(T) -> *mut u8) -> *mut MeshResult {
    match value {
        Ok(value) => mesh_alloc_result(0, encode(value)).cast(),
        Err(error) => mesh_alloc_result(1, string(&error)).cast(),
    }
}

unsafe fn inspect(
    handle: u64,
    borrowed: bool,
    namespace: *const MeshString,
    filter: *const MeshString,
) -> *mut MeshResult {
    result(
        jobs_testing::inspect(handle, borrowed, (*namespace).as_str(), (*filter).as_str()),
        |rows| string(&rows),
    )
}

#[no_mangle]
pub unsafe extern "C" fn mesh_test_jobs_inspect(
    pool: u64,
    namespace: *const MeshString,
    filter: *const MeshString,
) -> *mut MeshResult {
    inspect(pool, false, namespace, filter)
}

#[no_mangle]
pub unsafe extern "C" fn mesh_test_jobs_inspect_in(
    conn: u64,
    namespace: *const MeshString,
    filter: *const MeshString,
) -> *mut MeshResult {
    inspect(conn, true, namespace, filter)
}

unsafe fn assert_match(
    handle: u64,
    borrowed: bool,
    namespace: *const MeshString,
    filter: *const MeshString,
    wait_ms: i64,
    present: bool,
) -> *mut MeshResult {
    result(
        jobs_testing::assert_match(
            handle,
            borrowed,
            (*namespace).as_str(),
            (*filter).as_str(),
            wait_ms,
            present,
        ),
        |()| std::ptr::null_mut(),
    )
}

#[no_mangle]
pub unsafe extern "C-unwind" fn mesh_test_assert_enqueued(
    pool: u64,
    namespace: *const MeshString,
    filter: *const MeshString,
    wait_ms: i64,
) -> *mut MeshResult {
    assert_match(pool, false, namespace, filter, wait_ms, true)
}

#[no_mangle]
pub unsafe extern "C-unwind" fn mesh_test_refute_enqueued(
    pool: u64,
    namespace: *const MeshString,
    filter: *const MeshString,
    wait_ms: i64,
) -> *mut MeshResult {
    assert_match(pool, false, namespace, filter, wait_ms, false)
}

#[no_mangle]
pub unsafe extern "C" fn mesh_test_assert_enqueued_in(
    conn: u64,
    namespace: *const MeshString,
    filter: *const MeshString,
) -> *mut MeshResult {
    assert_match(conn, true, namespace, filter, 0, true)
}

#[no_mangle]
pub unsafe extern "C" fn mesh_test_refute_enqueued_in(
    conn: u64,
    namespace: *const MeshString,
    filter: *const MeshString,
) -> *mut MeshResult {
    assert_match(conn, true, namespace, filter, 0, false)
}

#[no_mangle]
pub unsafe extern "C" fn mesh_test_build_job(
    spec: *const MeshString,
    clock_ms: i64,
) -> *mut MeshResult {
    result(
        jobs_testing::build_job((*spec).as_str(), clock_ms),
        |context| string(&context),
    )
}

#[no_mangle]
pub unsafe extern "C-unwind" fn mesh_test_perform_job(
    spec: *const MeshString,
    clock_ms: i64,
    handler: *mut u8,
    env: *mut u8,
) -> *mut MeshResult {
    result(
        jobs_testing::perform_job((*spec).as_str(), clock_ms, handler, env),
        |outcome| string(&outcome),
    )
}
