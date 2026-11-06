#![cfg(unix)]
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};
#[path = "support/test_artifacts.rs"]
mod artifacts;
fn package() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/mesh-solana")
}
fn build(source: &str) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("transaction-read-proof");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("mesh.toml"),format!("[package]\nname=\"transaction-read-proof\"\nversion=\"0.1.0\"\n[dependencies]\nmesh-solana={{path={:?}}}\n",package())).unwrap();
    fs::write(project.join("main.mpl"), source).unwrap();
    let output = Command::new(artifacts::meshc_bin())
        .arg("build")
        .arg(&project)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (temp, project.join("transaction-read-proof"))
}
fn rpc(responses: Vec<Value>) -> (String, thread::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for response in responses {
            let deadline = Instant::now() + Duration::from_secs(40);
            let mut stream = loop {
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "RPC request never arrived");
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut block = [0; 8192];
            loop {
                let n = stream.read(&mut block).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&block[..n]);
                if let Some(offset) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..offset]);
                    let length = headers
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= offset + 4 + length {
                        requests.push(
                            serde_json::from_slice(&bytes[offset + 4..offset + 4 + length])
                                .unwrap(),
                        );
                        break;
                    }
                }
            }
            let body = response.to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        }
        requests
    });
    (url, handle)
}
#[test]
fn rpc_transport_rejects_mismatched_response_id() {
    let source = r#"from Solana.Read import rpc_request,rpc_send
fn main() do
 let client=Http.client()
 case rpc_request(41,"getSlot","[]") do
  Err(error)->println(error)
  Ok(request)->case rpc_send(client,Env.get("RPC_URL",""),request,10000,65536) do
    Ok(_)->println("rpc-id-accepted")
    Err(_)->println("rpc-id:ok")
   end
 end
end
"#;
    let (_temp, program) = build(source);
    let (url, server) = rpc(vec![json!({"jsonrpc":"2.0","id":42,"result":1})]);
    let output = Command::new(program).env("RPC_URL", url).output().unwrap();
    let requests = server.join().unwrap();
    assert_eq!(requests[0]["id"], 41);
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "rpc-id:ok\n");
}

#[test]
fn signed_wire_and_rpc_metadata_match_independent_sdk_fixtures() {
    let source = include_str!("../../../tests/e2e/solana_transaction_read.mpl");
    let (_temp, program) = build(source);
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/vectors/solana/finalized-transactions/fixtures.json");
    let output = Command::new(program)
        .env("FIXTURE", fixture)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "transaction-fixtures:ok\n",
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
