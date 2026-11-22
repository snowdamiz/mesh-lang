fn call_direct(input :: String) -> String!String do
  Test.perform_job(input, 1000, probe)
end

test("measure the compiled database-free direct handler helper") do
  let input = """{"worker":"proof","args":{"label":"direct-probe"}}"""
  case run_direct(input) do
    Ok(_) -> assert(true)
    Err(error) -> panic(error)
  end
end
