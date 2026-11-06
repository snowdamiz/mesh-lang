//! Same-client POSTs exercise the public runtime, not an application retry.
#[path = "support/http_persistence.rs"]
mod peer;

use mesh_rt::http::client::MeshClientResponse;
use mesh_rt::http::{
    mesh_http_body, mesh_http_build, mesh_http_client, mesh_http_client_close,
    mesh_http_max_redirects, mesh_http_send_with, mesh_http_timeout,
};
use mesh_rt::io::MeshResult;
use mesh_rt::string::{mesh_string_new, MeshString};

fn text(value: &str) -> *mut MeshString {
    mesh_string_new(value.as_ptr(), value.len() as u64)
}

fn check(peer: peer::Peer) {
    mesh_rt::gc::mesh_rt_init();
    let (url, server) = peer::serve(peer);
    let client = mesh_http_client();
    let mut replies = Vec::new();
    for body in ["first", "second"] {
        let request = mesh_http_build(text("post"), text(&url));
        mesh_http_body(request, text(body));
        mesh_http_timeout(request, 2_000);
        mesh_http_max_redirects(request, 0);
        let result = unsafe { &*(mesh_http_send_with(client, request) as *const MeshResult) };
        replies.push(if result.tag == 0 {
            let response = unsafe { &*(result.value as *const MeshClientResponse) };
            format!("{}:{}", response.status, unsafe {
                (*(response.body as *const MeshString)).as_str()
            })
        } else {
            format!("error:{}", unsafe {
                (*(result.value as *const MeshString)).as_str()
            })
        });
    }
    mesh_http_client_close(client);
    let seen = server.join().unwrap();
    assert_eq!(replies, ["200:one", "200:two"], "{seen:?}");
    peer::verify(peer, &seen);
}

#[test]
fn http10_length_delimited_response_closes_before_second_post() {
    check(peer::Peer::Http10);
}

#[test]
fn http10_explicit_keep_alive_reuses_connection() {
    check(peer::Peer::Http10KeepAlive);
}

#[test]
fn http11_default_persistence_reuses_connection() {
    check(peer::Peer::Http11);
}

#[test]
fn http11_explicit_close_opens_new_connection() {
    check(peer::Peer::Http11Close);
}
