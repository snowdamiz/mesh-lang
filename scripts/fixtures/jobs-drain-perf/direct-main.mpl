fn call_direct(input :: String) -> String!String do
  Ok(probe(input))
end

fn main() do
  let input = Env.get("J5_DIRECT_CONTEXT", "")
  case run_direct(input) do
    Ok(_) -> ()
    Err(error) ->
      println(error)
      Process.exit(1)
  end
end
