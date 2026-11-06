from Solana.Read import (
  RpcRequest,
  RpcResponse,
  get_block_time_request,
  block_time_from_response,
  rpc_request_json,
  rpc_response
)

fn proof() -> Bool!String do
  let request = get_block_time_request(4, U64.parse("18446744073709551615")?)?
  assert(rpc_request_json(request) == """{"jsonrpc":"2.0","id":4,"method":"getBlockTime","params":[18446744073709551615]}""")
  for value in ["0", "-1", "1574721591", "9223372036854775807", "-9223372036854775808"] do
    let result = block_time_from_response(rpc_response("{\"jsonrpc\":\"2.0\",\"id\":4,\"result\":#{value}}")?)?
    assert(result == String.to_int(value))
  end
  assert(block_time_from_response(rpc_response("""{"jsonrpc":"2.0","id":4,"result":null}""")?)? == None)
  for value in ["\"1574721591\"", "true", "1.5", "1e3", "[]", "{}", "9223372036854775808"] do
    case block_time_from_response(rpc_response("{\"jsonrpc\":\"2.0\",\"id\":4,\"result\":#{value}}")?) do
      Ok(_) -> do
        println("accepted invalid block time: #{value}")
        assert(false)
      end
      Err(_) -> nil
    end
  end
  let error = rpc_response("""{"jsonrpc":"2.0","id":4,"error":{"code":-32009,"message":"unavailable"}}""")?
  case block_time_from_response(error) do
    Ok(_) -> assert(false)
    Err(_) -> nil
  end
  for invalid in [
    """{"jsonrpc":"2.0","id":"4","result":1}""",
    """{"jsonrpc":"2.0","id":4.0,"result":1}""",
    """{"jsonrpc":"2.0","id":-1,"result":1}""",
    """{"jsonrpc":"2.0","id":4,"result":1,"error":{"code":-1}}""",
    """{"jsonrpc":"2.0","id":4}"""
  ] do
    case rpc_response(invalid) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  Ok(true)
end

test("block times preserve signed seconds, unknown time and malformed RPC types") do
  case proof() do
    Err(error) -> do
      println(error)
      assert(false)
    end
    Ok(value) -> assert(value)
  end
end
