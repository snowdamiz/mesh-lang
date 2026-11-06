#![cfg(unix)]

use std::{net::TcpListener, process::Command};

#[path = "support/test_artifacts.rs"]
mod artifacts;

#[test]
fn websocket_callbacks_keep_their_captured_heap_after_the_serving_actor_exits() {
    for optimized in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("ws-server-lifetime");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(
            project.join("main.mpl"),
            format!(
                r#"
actor serve(port :: Int, captured :: Bytes, closed :: String) do
  Ws.serve(fn(conn :: Int, _path :: String, _headers :: Map<String, String>) do
      Ws.send(conn, Bytes.to_hex(captured))
      1
    end,
    fn(conn :: Int, text :: String) do
      Ws.send(conn, Bytes.to_hex(captured) <> text)
      nil
    end,
    fn(_conn :: Int, _code :: Int, _reason :: String) do
      File.write(closed, Bytes.to_hex(captured))
      nil
    end,
    port)
end

fn churn(index :: Int) -> Int do
  if index == 4000 do
    index
  else
    let garbage = Bytes.repeat(index % 256, 256)
    churn(index + 1)
  end
end

fn closed_callback(path :: String, expected :: String, tries :: Int) -> Bool do
  if File.exists(path) do
    case File.read(path) do
      Ok(value) -> value == expected
      Err(_) -> false
    end
  else if tries == 0 do
    false
  else
    Timer.sleep(10)
    closed_callback(path, expected, tries - 1)
  end
end

fn proof() -> Bool!String do
  let captured = case Crypto.random_bytes(32) do
    Err(_) -> return Err("random failed")
    Ok(value) -> value
  end
  let expected = Bytes.to_hex(captured)
  let _server = spawn(serve, {port}, captured, "{closed}")
  Timer.sleep(150)
  churn(0)
  let options = WsClient.options()
    |> WsClient.connect_timeout(2000)
    |> WsClient.max_message_bytes(256)
    |> WsClient.queue_capacity(8)
  let connection = WsClient.connect("ws://127.0.0.1:{port}/", options)?
  let first = WsClient.recv(connection, 2000)?
  if first.kind != "text" || Bytes.to_utf8(first.data)? != expected do
    return Err("captured on_connect value lost")
  end
  WsClient.send_text(connection, "-message")?
  let second = WsClient.recv(connection, 2000)?
  if second.kind != "text" || Bytes.to_utf8(second.data)? != expected <> "-message" do
    return Err("captured on_message value lost")
  end
  WsClient.close(connection, 1000, "done")?
  if !closed_callback("{closed}", expected, 100) do
    return Err("captured on_close value lost")
  end
  Ok(true)
end

fn main() do
  case proof() do
    Ok(true) -> println("survived")
    _ -> println("capture lost")
  end
end
"#,
                closed = temp.path().join("closed").display()
            ),
        )
        .unwrap();
        let mut command = Command::new(artifacts::meshc_bin());
        command.args(["build", project.to_str().unwrap()]);
        if optimized {
            command.args(["--opt-level", "2"]);
        }
        let build = command.output().unwrap();
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );
        let run = Command::new(project.join("ws-server-lifetime"))
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&run.stdout),
            "survived\n",
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
    }
}
