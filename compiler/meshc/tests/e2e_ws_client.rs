#![cfg(unix)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;

use mesh_rt::ws::handshake::compute_accept_key;
use mesh_rt::ws::{read_frame, write_frame, WsOpcode};

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

#[test]
fn mesh_websocket_client_exchanges_text_binary_and_close_frames() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        let mut request = Vec::new();
        let mut byte = [0u8; 1];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        let request = String::from_utf8(request).unwrap();
        let key = request
            .lines()
            .find_map(|line| {
                line.split_once(':')
                    .filter(|(name, _)| name.eq_ignore_ascii_case("Sec-WebSocket-Key"))
                    .map(|(_, value)| value.trim())
            })
            .unwrap();
        write!(
            stream,
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
            compute_accept_key(key)
        )
        .unwrap();
        stream.flush().unwrap();

        let text = read_frame(&mut stream).unwrap();
        assert_eq!(text.opcode, WsOpcode::Text);
        assert_eq!(text.payload, b"subscribe");
        let binary = read_frame(&mut stream).unwrap();
        assert_eq!(binary.opcode, WsOpcode::Binary);
        assert_eq!(binary.payload, [1, 2]);

        write_frame(&mut stream, WsOpcode::Text, b"ready", true).unwrap();
        write_frame(&mut stream, WsOpcode::Binary, &[1], false).unwrap();
        write_frame(&mut stream, WsOpcode::Continuation, &[2, 3], true).unwrap();
        let close = read_frame(&mut stream).unwrap();
        assert_eq!(close.opcode, WsOpcode::Close);
        assert_eq!(&close.payload[..2], &1000u16.to_be_bytes());
        assert_eq!(&close.payload[2..], b"done");
    });

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("ws-client");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("main.mpl"),
        format!(
            r##"
fn print_message(message :: WsMessage) do
  if message.kind == "text" do
    case Bytes.to_utf8(message.data) do
      Ok(text) -> println(text)
      Err(error) -> println(error)
    end
  else if message.kind == "binary" do
    println(Bytes.to_hex(message.data))
  else
    println("close:#{{message.close_code}}:#{{message.close_reason}}")
  end
end

fn exchange(connection :: Int) -> Int!String do
  WsClient.send_text(connection, "subscribe")?
  (("0102" |> Bytes.from_hex())? |2> WsClient.send_bytes(connection))?
  let first = WsClient.recv(connection, 5_000)?
  let second = WsClient.recv(connection, 5_000)?
  print_message(first)
  print_message(second)
  WsClient.close(connection, 1_000, "done")?
  Ok(0)
end

fn main() do
  let options = WsClient.options()
    |> WsClient.connect_timeout(5_000)
    |> WsClient.heartbeat_timeout(5_000)
    |> WsClient.max_message_bytes(1_024)
    |> WsClient.queue_capacity(8)
  case WsClient.connect("ws://127.0.0.1:{port}/feed", options) do
    Ok(connection) -> case exchange(connection) do
      Ok(_) -> println("done")
      Err(error) -> println("error:" <> error)
    end
    Err(error) -> println("error:" <> error)
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
    let run = Command::new(project.join("ws-client")).output().unwrap();
    assert!(
        run.status.success(),
        "Mesh WebSocket client failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "ready\n010203\ndone\n"
    );
    server.join().unwrap();
}
