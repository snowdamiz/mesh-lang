fn main() do
  let body = "{\"quoted\":\"42\",\"number\":42,\"payload\":{\"atoms\":\"7\"}}"
  let payload = Json.get(body, "payload")
  println("${Json.is_string(body, "quoted")}")
  println("${Json.is_string(body, "number")}")
  println("${Json.is_string(payload, "atoms")}")
end
