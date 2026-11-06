from Solana.Read import (
  Pubkey,
  AccountInfo,
  AccountAtSlot,
  RpcRequest,
  RpcResponse,
  pubkey,
  get_account_info_slice_request,
  rpc_request_json,
  rpc_response,
  account_from_response
)

fn proof() -> Bool!String do
  let address = pubkey("11111111111111111111111111111111")?
  let request = get_account_info_slice_request(7,
    address,
    "finalized",
    64,
    104,
    Some(U64.parse("18446744073709551615")?))?
  let encoded = rpc_request_json(request)
  assert(String.contains(encoded, "\"dataSlice\":{\"offset\":64,\"length\":104}"))
  assert(String.contains(encoded, "\"minContextSlot\":18446744073709551615"))
  assert(String.contains(encoded, "\"encoding\":\"base64\""))
  for (offset, length) in [(-1, 64), (0, -1), (10485760, 1), (9223372036854775807, 64)] do
    case get_account_info_slice_request(1, address, "finalized", offset, length, None) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  case get_account_info_slice_request(1, address, "fast", 0, 64, None) do
    Ok(_) -> assert(false)
    Err(_) -> nil
  end
  let response = rpc_response("""{"jsonrpc":"2.0","id":7,"result":{"context":{"slot":42},"value":{"data":["AQID","base64"],"executable":false,"lamports":1,"owner":"11111111111111111111111111111111","rentEpoch":18446744073709551615}}}""")?
  let account = account_from_response(response)?
  assert(U64.to_string(account.slot) == "42")
  assert(Bytes.to_hex(account.account.data) == "010203")
  assert(U64.to_string(account.account.rent_epoch) == "18446744073709551615")
  for invalid in ["""{"context":{"slot":42},"value":null}""", """{"context":{},"value":null}"""] do
    case account_from_response(%{response | result_json: invalid}) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  case account_from_response(%{response | ok: false, error_json: "unavailable"}) do
    Ok(_) -> assert(false)
    Err(_) -> nil
  end
  Ok(true)
end

test("account slices use bounded byte ranges and preserve the RPC context and u64 metadata") do
  case proof() do
    Err(error) -> do
      println(error)
      assert(false)
    end
    Ok(value) -> assert(value)
  end
end
