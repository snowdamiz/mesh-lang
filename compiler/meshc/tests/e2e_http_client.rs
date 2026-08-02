#![cfg(unix)]

#[path = "support/test_artifacts.rs"]
mod artifacts;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

fn meshc_bin() -> PathBuf {
    let mut path = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    if path.file_name().is_some_and(|name| name == "deps") {
        path.pop();
    }
    path.join("meshc")
}

fn read_request(stream: &mut impl Read) -> String {
    let mut request = Vec::new();
    let mut byte = [0u8; 1];
    while !request.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).unwrap();
        request.push(byte[0]);
    }
    String::from_utf8(request).unwrap()
}

#[test]
fn mesh_http_client_uses_pooling_bounds_and_metrics_end_to_end() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        let first = read_request(&mut stream);
        assert!(first.starts_with("GET /funding?market=SOL%2FUSDC HTTP/1.1\r\n"));
        assert!(first
            .lines()
            .any(|line| line.eq_ignore_ascii_case("x-test: mesh")));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: keep-alive\r\n\r\none")
            .unwrap();

        let second = read_request(&mut stream);
        assert!(second.starts_with("GET /funding?market=SOL%2FUSDC HTTP/1.1\r\n"));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\ntwo")
            .unwrap();
    });

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("http-client");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("main.mpl"),
        format!(
            r##"
fn request(url :: String) do
  Http.build(:get, url)
    |> Http.header("X-Test", "mesh")
    |> Http.query("market", "SOL/USDC")
    |> Http.timeout(5_000)
    |> Http.stage_timeout(:resolve, 1_000)
    |> Http.stage_timeout(:connect, 1_000)
    |> Http.stage_timeout(:send, 1_000)
    |> Http.stage_timeout(:first_byte, 1_000)
    |> Http.stage_timeout(:body, 1_000)
    |> Http.max_response_bytes(16)
end

fn fetch(client :: Int, url :: String) do
  case request(url) |2> Http.send_with(client) do
    Ok(response) -> println(response.body)
    Err(error) -> println("error:" <> error)
  end
end

fn main() do
  let client = Http.client()
  let url = "http://127.0.0.1:{port}/funding"
  fetch(client, url)
  fetch(client, url)
  Http.client_close(client)
  println(Http.retry_class(:get, "TIMEOUT_CONNECT: timed out"))
  println(Http.retry_class(:post, "TIMEOUT_CONNECT: timed out"))
  let metrics = Http.metrics()
  println("#{{metrics.requests}}:#{{metrics.in_flight}}:#{{metrics.response_bytes}}")
  println("#{{metrics.dns_micros}}:#{{metrics.connect_micros}}:#{{metrics.tls_micros}}:#{{metrics.first_byte_micros}}:#{{metrics.total_micros}}")
end
"##
        ),
    )
    .unwrap();

    let build = Command::new(meshc_bin())
        .args(["build", project.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(project.join("http-client")).output().unwrap();
    assert!(
        run.status.success(),
        "Mesh HTTP client failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8(run.stdout).unwrap();
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 6, "{stdout}");
    assert_eq!(
        &lines[..5],
        ["one", "two", "safe_retry", "unsafe_retry", "2:0:6"]
    );
    let timings = lines[5]
        .split(':')
        .map(|value| value.parse::<u64>().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(timings.len(), 5);
    assert!(timings[0] > 0, "DNS timing was not recorded");
    assert!(timings[1] > 0, "connect timing was not recorded");
    assert_eq!(timings[2], 0, "plain HTTP unexpectedly recorded TLS time");
    assert!(timings[3] > 0, "first-byte timing was not recorded");
    assert!(timings[4] >= timings[3], "total time preceded first byte");
    server.join().unwrap();
}

#[test]
fn mesh_http_client_preserves_binary_request_and_response_bodies() {
    artifacts::ensure_mesh_rt_staticlib();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let headers = read_request(&mut stream);
        assert!(headers.starts_with("POST /binary HTTP/1.1\r\n"));
        assert!(headers
            .lines()
            .any(|line| line.eq_ignore_ascii_case("content-length: 3")));
        let mut body = [0u8; 3];
        stream.read_exact(&mut body).unwrap();
        assert_eq!(body, [0x00, 0xff, 0x80]);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 3\r\nConnection: close\r\n\r\n\x00\xff\x80",
            )
            .unwrap();
    });

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("http-binary-client");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("main.mpl"),
        format!(
            r##"
fn main() do
  case Bytes.from_list([0, 255, 128]) do
    Err(_) -> println("fixture:error")
    Ok(body) -> do
      let request = Http.build(:post, "http://127.0.0.1:{port}/binary")
        |> Http.body_bytes(body)
        |> Http.max_response_bytes(3)
      case Http.send(request) do
        Err(error) -> println("http:error:" <> error)
        Ok(response) -> println(Bytes.to_hex(response.body_bytes))
      end
    end
  end
end
"##
        ),
    )
    .unwrap();

    let build = Command::new(meshc_bin())
        .args(["build", project.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(project.join("http-binary-client"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "Mesh HTTP client failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "00ff80\n");
    server.join().unwrap();
}
