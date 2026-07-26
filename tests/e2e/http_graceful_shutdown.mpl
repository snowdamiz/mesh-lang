fn health(_request :: Request) -> Response do
  HTTP.response(200, "ok")
end

fn main() do
  Process.request_shutdown()
  HTTP.router()
    |> HTTP.on_get("/health", health)
    |> HTTP.serve(0)
  println("stopped")
end
