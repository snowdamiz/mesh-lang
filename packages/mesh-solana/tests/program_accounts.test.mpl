from Solana.Read import (
  Pubkey,
  AccountInfo,
  KeyedAccountInfo,
  ProgramAccountsAtSlot,
  RpcRequest,
  RpcResponse,
  ProgramAccountFilter,
  pubkey,
  data_size_filter,
  memcmp_filter,
  program_accounts_context_request,
  program_accounts_from_response,
  rpc_request_json,
  rpc_response
)

fn proof() -> Bool!String do
  let address = pubkey("11111111111111111111111111111111")?
  let request = program_accounts_context_request(9,
    address,
    [data_size_filter(320)?, memcmp_filter(5, "2")?],
    "finalized",
    Some(U64.parse("18446744073709551615")?))?
  assert(rpc_request_json(request) == """{"jsonrpc":"2.0","id":9,"method":"getProgramAccounts","params":["11111111111111111111111111111111",{"commitment":"finalized","encoding":"base64","filters":[{"dataSize":320},{"memcmp":{"offset":5,"bytes":"2"}}],"withContext":true,"minContextSlot":18446744073709551615}]}""")
  assert(!String.contains(rpc_request_json(program_accounts_context_request(1,
      address,
      [],
      "confirmed",
      None)?),
    "minContextSlot"))
  case program_accounts_context_request(1,
    Pubkey { bytes: Bytes.empty() },
    [],
    "finalized",
    None) do
    Ok(_) -> assert(false)
    Err(_) -> nil
  end
  case program_accounts_context_request(1, address, [], "fast", None) do
    Ok(_) -> assert(false)
    Err(_) -> nil
  end
  let account = """{"data":["AQID","base64"],"owner":"11111111111111111111111111111111","lamports":18446744073709551615,"executable":false,"rentEpoch":1}"""
  let item = """{"pubkey":"11111111111111111111111111111111","account":#{account}}"""
  let response = rpc_response("""{"jsonrpc":"2.0","id":9,"result":{"context":{"slot":18446744073709551615},"value":[#{item}]}}""")?
  let found = program_accounts_from_response(response, 64)?
  assert(U64.to_string(found.slot) == "18446744073709551615" && List.length(found.accounts) == 1)
  let first = List.get(found.accounts, 0)
  assert(Bytes.secure_equals(first.address.bytes, address.bytes))
  assert(Bytes.to_hex(first.account.data) == "010203"
    && U64.to_string(first.account.lamports) == "18446744073709551615")
  let empty = program_accounts_from_response(%{response |
      result_json: """{"context":{"slot":7},"value":[]}"""
    },
    0)?
  assert(List.length(empty.accounts) == 0 && U64.to_string(empty.slot) == "7")
  for limit in [-1, 0, 1001] do
    case program_accounts_from_response(response, limit) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  for invalid in [
    """{"context":{"slot":1},"value":[#{item},#{item}]}""",
    """{"context":{"slot":"1"},"value":[#{item}]}""",
    """{"context":{"slot":-1},"value":[]}""",
    """{"context":{"slot":1.5},"value":[]}""",
    """{"context":{"slot":1},"value":[null]}""",
    """{"context":{"slot":1},"value":[{"pubkey":"bad","account":#{account}}]}""",
    """{"context":{"slot":1},"value":[{"pubkey":"11111111111111111111111111111111","account":null}]}""",
    """{"context":{"slot":1},"value":[{}]}""",
    """{"context":{"slot":1},"value":null}""",
    "[]"
  ] do
    case program_accounts_from_response(%{response | result_json: invalid}, 64) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  case program_accounts_from_response(%{response | ok: false, error_json: "unavailable"}, 64) do
    Ok(_) -> assert(false)
    Err(_) -> nil
  end
  Ok(true)
end

test("context-aware program account discovery is bounded, unsigned and rejects duplicate or malformed records") do
  case proof() do
    Err(error) -> do
      println(error)
      assert(false)
    end
    Ok(value) -> assert(value)
  end
end
