//! After the library shuts down there is no runtime to start again, so no
//! fixture can be installed. (A process of its own: shutdown is final.)

use mesh_rt::library::{mesh_library_init, mesh_library_shutdown, MESH_LIBRARY_OK};
use mesh_test_rt::{mesh_test_install_in_memory_secure_store, mesh_test_set_push_token};

#[test]
fn fixtures_need_a_running_library() {
    assert_eq!(mesh_library_init(), MESH_LIBRARY_OK);
    assert_eq!(mesh_library_shutdown(), MESH_LIBRARY_OK);

    assert_eq!(mesh_test_install_in_memory_secure_store(), 0);
    let selector = mesh_rt::bytes::mesh_bytes_new(b"expo/v1".as_ptr(), 7);
    let token = mesh_rt::bytes::mesh_bytes_new(b"token".as_ptr(), 5);
    assert_eq!(mesh_test_set_push_token(selector, token), 0);
}
