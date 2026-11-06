use super::*;
use crate::http::trust_roots::fixture::TlsFixture;

fn peer(
    fixture: &TlsFixture,
    expired: bool,
    expected: usize,
    keep_alive: bool,
) -> (u16, std::thread::JoinHandle<(usize, usize)>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    (port, fixture.serve(listener, expired, expected, keep_alive))
}

fn request(url: String, roots: Option<TrustRoots>) -> MeshRequestData {
    let mut request = MeshRequestData::new("get", &url);
    request.timeouts.global = Duration::from_secs(2);
    request.max_redirects = Some(0);
    request.trust_roots = roots;
    request
}

#[test]
fn http_trust_root_validates_chain_hostname_and_expired_leaf() {
    let fixture = TlsFixture::new();
    let unrelated = TlsFixture::new();
    let roots = parse_root_pem(&fixture.pem("ca.pem")).unwrap();
    let wrong = parse_root_pem(&unrelated.pem("ca.pem")).unwrap();
    let (port, server) = peer(&fixture, false, 4, false);
    let url = format!("https://localhost:{port}/");
    let response = execute_request(
        &http_agent(),
        request(url.clone(), Some(Arc::clone(&roots))),
        None,
    )
    .unwrap();
    assert_eq!(response.body, b"ok");
    for request in [
        request(url.clone(), None),
        request(url, Some(wrong)),
        request(
            format!("https://127.0.0.1:{port}/"),
            Some(Arc::clone(&roots)),
        ),
    ] {
        let error = execute_request(&http_agent(), request, None).unwrap_err();
        assert!(error.starts_with("TLS_ERROR: "), "{error}");
    }
    assert_eq!(server.join().unwrap(), (4, 1));
    let (port, expired) = peer(&fixture, true, 1, false);
    let error = execute_request(
        &http_agent(),
        request(format!("https://localhost:{port}/"), Some(roots)),
        None,
    )
    .unwrap_err();
    assert!(error.starts_with("TLS_ERROR: "), "{error}");
    assert_eq!(expired.join().unwrap(), (1, 0));
}

#[test]
fn http_trust_root_cannot_reuse_a_connection_from_another_root_policy() {
    let fixture = TlsFixture::new();
    let unrelated = TlsFixture::new();
    let roots = parse_root_pem(&fixture.pem("ca.pem")).unwrap();
    let wrong = parse_root_pem(&unrelated.pem("ca.pem")).unwrap();
    let (port, server) = peer(&fixture, false, 2, true);
    let url = format!("https://localhost:{port}/");
    let pooled = agent_for_request(&http_agent(), &request(url.clone(), Some(roots)));
    assert_eq!(
        execute_request(&pooled, request(url.clone(), None), None)
            .unwrap()
            .body,
        b"ok"
    );
    let error = execute_request(&pooled, request(url, Some(wrong)), None).unwrap_err();
    assert!(error.starts_with("TLS_ERROR: "), "{error}");
    drop(pooled);
    assert_eq!(server.join().unwrap(), (2, 1));
}

#[test]
fn http_trust_root_rejects_plain_http_and_permanently_poisons_bad_builders() {
    // Share the existing API registry lock with the capacity-bound tests.
    let _api = super::tests::API.lock();
    crate::gc::mesh_rt_init();
    let fixture = TlsFixture::new();
    let roots = parse_root_pem(&fixture.pem("ca.pem")).unwrap();
    let error = execute_request(
        &http_agent(),
        request("http://127.0.0.1:1/".into(), Some(roots)),
        None,
    )
    .unwrap_err();
    assert!(error.contains("explicit TLS roots require HTTPS"));
    let handle = mesh_http_build(mesh_str("get"), mesh_str("https://127.0.0.1:1/"));
    let empty = mesh_bytes_new([].as_ptr(), 0);
    assert_eq!(mesh_http_trust_root_pem(handle, empty), handle);
    let pem = fixture.pem("ca.pem");
    let pem = mesh_bytes_new(pem.as_ptr(), pem.len() as u64);
    assert_eq!(mesh_http_trust_root_pem(handle, pem), handle);
    let consumed = take_request(handle).unwrap();
    assert!(validate_request(&consumed)
        .unwrap_err()
        .starts_with("INVALID_TLS_ROOTS:"));
    assert!(take_request(handle).is_err());
    assert_eq!(mesh_http_trust_root_pem(handle, pem), 0);
}

#[test]
fn http_trust_root_refuses_https_redirects_to_plain_http() {
    let fixture = TlsFixture::new();
    let roots = parse_root_pem(&fixture.pem("ca.pem")).unwrap();
    let target = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    let target_port = target.local_addr().unwrap().port();
    let redirect = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = redirect.local_addr().unwrap().port();
    let server = fixture.serve_reply(redirect, false, 1, false,
        format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: http://127.0.0.1:{target_port}/untrusted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes());
    let mut request = request(format!("https://localhost:{port}/"), Some(roots));
    request.max_redirects = Some(3);
    request
        .headers
        .push(("Authorization".into(), "owned-test-only".into()));
    assert!(execute_request(&http_agent(), request, None).is_err());
    assert_eq!(server.join().unwrap(), (1, 1));
    assert_eq!(
        target.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
