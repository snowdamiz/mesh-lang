from Solana.Read import (
  Pubkey,
  AccountInfo,
  OptionalAccountsAtSlot,
  RpcRequest,
  RpcResponse,
  pubkey,
  get_multiple_accounts_request,
  get_multiple_accounts_context_request,
  rpc_request_json,
  rpc_response,
  optional_accounts_from_response,
  multiple_accounts_from_response
)

fn proof() -> Bool!String do
  let address = pubkey("11111111111111111111111111111111")?
  let request = get_multiple_accounts_context_request(7,
    [address],
    "finalized",
    Some(U64.parse("18446744073709551615")?))?
  assert(rpc_request_json(request) == """{"jsonrpc":"2.0","id":7,"method":"getMultipleAccounts","params":[["11111111111111111111111111111111"],{"commitment":"finalized","encoding":"base64","minContextSlot":18446744073709551615}]}""")
  let hundred = for index in 0..100 do
    address
  end
  assert(get_multiple_accounts_context_request(1,
    hundred,
    "confirmed",
    None)?.method == "getMultipleAccounts")
  for addresses in [[], List.append(hundred, address), [Pubkey { bytes: Bytes.empty() }]] do
    case get_multiple_accounts_request(1, addresses, "finalized") do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  for commitment in ["", "fast"] do
    case get_multiple_accounts_context_request(1, [address], commitment, None) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  let account = """{"data":["AQID","base64"],"owner":"11111111111111111111111111111111","lamports":1,"executable":false,"rentEpoch":18446744073709551615}"""
  for malformed in [
    String.replace(account, "\"lamports\":1", "\"lamports\":\"1\""),
    String.replace(account,
      "\"rentEpoch\":18446744073709551615",
      "\"rentEpoch\":\"18446744073709551615\"")
  ] do
    let response = rpc_response("""{"jsonrpc":"2.0","id":7,"result":{"context":{"slot":1},"value":[#{malformed}]}}""")?
    case optional_accounts_from_response(response) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  let response = rpc_response("""{"jsonrpc":"2.0","id":7,"result":{"context":{"slot":18446744073709551615},"value":[null,#{account},null]}}""")?
  let batch = optional_accounts_from_response(response)?
  assert(U64.to_string(batch.slot) == "18446744073709551615" && List.length(batch.accounts) == 3)
  case List.get(batch.accounts, 0) do
    None -> nil
    Some(_) -> assert(false)
  end
  case List.get(batch.accounts, 1) do
    None -> assert(false)
    Some(value) -> assert(Bytes.to_hex(value.data) == "010203")
  end
  case List.get(batch.accounts, 2) do
    None -> nil
    Some(_) -> assert(false)
  end
  case multiple_accounts_from_response(response) do
    Ok(_) -> assert(false)
    Err(_) -> nil
  end
  for invalid in [
    """{"context":{"slot":-1},"value":[null]}""",
    """{"context":{"slot":"1"},"value":[null]}""",
    """{"context":{"slot":1.5},"value":[null]}""",
    """{"context":{"slot":1},"value":[{}]}""",
    """{"context":{"slot":1},"value":null}""",
    """{"context":{"slot":1},"value":["null"]}"""
  ] do
    case optional_accounts_from_response(%{response | result_json: invalid}) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  case optional_accounts_from_response(%{response | ok: false, error_json: "unavailable"}) do
    Ok(_) -> assert(false)
    Err(_) -> nil
  end
  Ok(true)
end

test("atomic account batches preserve missing positions and unsigned contexts without accepting malformed accounts") do
  case proof() do
    Err(error) -> do
      println(error)
      assert(false)
    end
    Ok(value) -> assert(value)
  end
end
