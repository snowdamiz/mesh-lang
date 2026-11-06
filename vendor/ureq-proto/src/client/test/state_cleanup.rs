use http::{Request, Response, StatusCode, Version};

use crate::client::test::scenario::write_response;
use crate::CloseReason;

use super::scenario::Scenario;

#[test]
fn reuse_without_send_body() {
    let scenario = Scenario::builder()
        .get("https://a.test")
        .response(Response::new(()))
        .recv_body("hello", false)
        .build();

    let call = scenario.to_cleanup();

    assert!(!call.must_close_connection());
}

#[test]
fn reuse_with_send_body() {
    let scenario = Scenario::builder()
        .post("https://a.test")
        .send_body("hi", false)
        .response(Response::new(()))
        .recv_body("hello", false)
        .build();

    let call = scenario.to_cleanup();

    assert!(!call.must_close_connection());
}

#[test]
fn reuse_without_recv_body() {
    let scenario = Scenario::builder()
        .head("https://a.test")
        .response(Response::new(()))
        .build();

    let call = scenario.to_cleanup();

    assert!(!call.must_close_connection());
}

#[test]
fn reuse_after_redirect() {
    let scenario = Scenario::builder()
        .get("https://a.test")
        .redirect(StatusCode::FOUND, "https://b.test")
        .build();

    let call = scenario.to_cleanup();

    assert!(!call.must_close_connection());
}

#[test]
fn close_due_to_http10() {
    let scenario = Scenario::builder()
        .request(
            Request::get("https://a.test")
                .version(Version::HTTP_10)
                .body(())
                .unwrap(),
        )
        .build();

    let call = scenario.to_cleanup();
    let inner = call.inner();
    assert_eq!(
        *inner.close_reason.first().unwrap(),
        CloseReason::CloseDelimitedBody
    );

    assert!(call.must_close_connection());
}

#[test]
fn close_due_to_client_connection_close() {
    let scenario = Scenario::builder()
        .get("https://a.test")
        .header("connection", "close")
        .build();

    let call = scenario.to_cleanup();
    let inner = call.inner();
    assert_eq!(
        *inner.close_reason.first().unwrap(),
        CloseReason::ClientConnectionClose
    );

    assert!(call.must_close_connection());
}

#[test]
fn close_due_to_server_connection_close() {
    let scenario = Scenario::builder()
        .get("https://a.test")
        .response(
            Response::builder()
                .header("connection", "close")
                .body(())
                .unwrap(),
        )
        .build();

    let call = scenario.to_cleanup();
    let inner = call.inner();
    assert_eq!(
        *inner.close_reason.first().unwrap(),
        CloseReason::ServerConnectionClose
    );

    assert!(call.must_close_connection());
}

#[test]
fn close_due_to_not_100_continue() {
    let scenario = Scenario::builder()
        .post("https://q.test")
        .header("expect", "100-continue")
        .send_body("hi", false)
        .build();

    let mut call = scenario.to_await_100();

    let input = write_response(
        &Response::builder()
            .status(StatusCode::FORBIDDEN)
            .body(())
            .unwrap(),
    );
    call.try_read_100(&input).unwrap();

    let inner = call.inner();
    assert_eq!(
        *inner.close_reason.first().unwrap(),
        CloseReason::Not100Continue
    );
}

#[test]
fn close_due_to_close_delimited_body() {
    // no content-length or transfer-encoding
    let scenario = Scenario::builder().get("https://a.test").build();

    let call = scenario.to_cleanup();
    let inner = call.inner();
    assert_eq!(
        *inner.close_reason.first().unwrap(),
        CloseReason::CloseDelimitedBody
    );

    assert!(call.must_close_connection());
}

#[test]
fn http10_response_without_keep_alive_is_not_reusable() {
    for (method, status, length, body) in [
        ("POST", StatusCode::OK, "3", "one"),
        ("POST", StatusCode::OK, "0", ""),
        ("HEAD", StatusCode::OK, "999", ""),
        ("GET", StatusCode::NO_CONTENT, "0", ""),
        ("GET", StatusCode::NOT_MODIFIED, "999", ""),
        ("GET", StatusCode::FOUND, "0", ""),
    ] {
        let mut response = Response::builder().version(Version::HTTP_10).status(status);
        if body.is_empty() {
            response = response.header("content-length", length);
        }
        if status == StatusCode::FOUND {
            response = response.header("location", "https://b.test/");
        }
        let scenario = Scenario::builder()
            .request(
                Request::builder()
                    .method(method)
                    .uri("https://a.test/")
                    .body(())
                    .unwrap(),
            )
            .send_body("", false)
            .response(response.body(()).unwrap());
        let scenario = if body.is_empty() {
            scenario.build()
        } else {
            scenario.recv_body(body, false).build()
        };
        assert!(
            scenario.to_cleanup().must_close_connection(),
            "{method} {status} length={length}"
        );
    }
}

#[test]
fn response_persistence_connection_tokens_and_precedence() {
    for (version, headers, close) in [
        (Version::HTTP_10, vec![], true),
        (Version::HTTP_10, vec!["Keep-Alive"], false),
        (Version::HTTP_10, vec!["upgrade, keep-alive"], false),
        (Version::HTTP_10, vec!["keep-alive", "CLOSE"], true),
        (Version::HTTP_10, vec!["Keep-Alive, close"], true),
        (Version::HTTP_10, vec!["x-keep-alive"], true),
        (Version::HTTP_11, vec![], false),
        (Version::HTTP_11, vec!["Keep-Alive"], false),
        (Version::HTTP_11, vec!["keep-alive, CLOSE"], true),
    ] {
        let mut response = Response::builder().version(version);
        for value in headers.iter() {
            response = response.header("connection", *value);
        }
        let scenario = Scenario::builder()
            .post("https://a.test/")
            .send_body("request", false)
            .response(response.body(()).unwrap())
            .recv_body("response", false)
            .build();
        assert_eq!(
            scenario.to_cleanup().must_close_connection(),
            close,
            "{version:?} {headers:?}"
        );
    }
}

#[test]
fn http10_does_not_add_a_reason_when_closure_is_already_required() {
    for request_version in [Version::HTTP_10, Version::HTTP_11] {
        let scenario = Scenario::builder()
            .request(
                Request::post("https://a.test/")
                    .version(request_version)
                    .body(())
                    .unwrap(),
            )
            .header("connection", "close")
            .header("expect", "100-continue")
            .send_body("request", false)
            .build();
        let mut waiting = scenario.to_await_100();
        let response = b"HTTP/1.0 403 Forbidden\r\n\r\n";
        assert_eq!(waiting.try_read_100(response).unwrap(), 0);
        let mut receiving = match waiting.proceed().unwrap() {
            crate::client::Await100Result::RecvResponse(call) => call,
            _ => panic!("early refusal must receive a response"),
        };
        receiving.try_response(response, false).unwrap();
        let body = match receiving.proceed().unwrap() {
            crate::client::RecvResponseResult::RecvBody(call) => call,
            _ => panic!("an unframed refusal has a close-delimited body"),
        };
        // Existing request/expect/body reasons must fit the original 4 slots.
        assert!(body.inner().close_reason.len() <= 4);
        let cleanup = match body.proceed().unwrap() {
            crate::client::RecvBodyResult::Cleanup(call) => call,
            _ => panic!("refusal must clean up"),
        };
        assert!(cleanup.must_close_connection());
    }
}
