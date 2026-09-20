use mesh_typeck::check;

#[test]
fn websocket_client_surface_type_checks_with_pipe_configuration() {
    let parsed = mesh_parser::parse(
        r##"
fn print_message(message :: WsMessage) do
  println(message.kind)
  println("#{Bytes.length(message.data)}")
end

fn main() do
  let options = WsClient.options()
    |> WsClient.connect_timeout(2_000)
    |> WsClient.heartbeat_timeout(30_000)
    |> WsClient.max_message_bytes(1_048_576)
    |> WsClient.queue_capacity(64)

  let delay = WsClient.reconnect_delay(3, 100, 10_000, 100_000)
  case delay do
    Ok(milliseconds) -> println("#{milliseconds}")
    Err(error) -> println(error)
  end

  case WsClient.connect("wss://example.com/feed", options) do
    Ok(connection) -> case WsClient.recv(connection, 5_000) do
        Ok(message) -> print_message(message)
        Err(error) -> println(error)
      end
    Err(error) -> println(error)
  end
end
"##,
    );
    assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
    let result = check(&parsed);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
}
