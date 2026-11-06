fn fetch(client :: Int, sequence :: String) do
  let request = Http.build(:post, Env.get("MESH_HTTP_PERSISTENCE_URL", ""))
    |> Http.body(sequence)
    |> Http.timeout(2_000)
    |> Http.max_redirects(0)
    |> Http.max_response_bytes(16)
  case Http.send_with(client, request) do
    Ok(response) -> println("#{response.status}:#{response.body}")
    Err(error) -> println("error:" <> error)
  end
end

fn main() do
  let client = Http.client()
  fetch(client, "first")
  fetch(client, "second")
  Http.client_close(client)
end
