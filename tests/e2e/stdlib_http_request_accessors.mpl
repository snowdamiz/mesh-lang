fn or_none(value :: Option<String>) -> String do
  case value do
    Some(text) -> text
    None -> "none"
  end
end

fn echo_handler(request) do
  let agent = or_none(Request.header(request, "x-agent"))
  let page = or_none(Request.query(request, "page"))
  let missing = or_none(Request.query(request, "missing"))
  let key = or_none(HTTP.idempotency_key(request))
  let id = HTTP.request_id(request)
  let bytes = Bytes.length(Request.body_bytes(request))
  HTTP.response(200,
    "#{Request.method(request)} #{Request.path(request)} agent=#{agent} page=#{page} missing=#{missing} key=#{key} id=#{String.length(id) > 0} body=#{Request.body(request)} bytes=#{bytes}")
end

fn main() do
  let r = HTTP.router()
  let r = HTTP.route(r, "/*", echo_handler)
  HTTP.serve(r, 18083)
end
