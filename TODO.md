1. Allow pipe operator to choose which argument to pass result to instead of forcing once way like elixir does

syntax could look something like this:
old: |>
new: |2> or |3>

2. Add full regex support

3. Review string strangeness

let ws_port_str = get_env_or_default("MESHER_WS_PORT", "8081")
let ws_port = parse_port(ws_port_str, 8081)
let http_port_str = get_env_or_default("MESHER_HTTP_PORT", "8080")
let http_port = parse_port(http_port_str, 8080)
