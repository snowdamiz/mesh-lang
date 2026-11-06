from Solana.Read import (
  Pubkey,
  AccountInfo,
  AccountSlice,
  AccountSliceAtSlot,
  OptionalAccountSlicesAtSlot,
  RpcRequest,
  RpcResponse,
  pubkey,
  rpc_response,
  rpc_request_json,
  get_multiple_accounts_slice_request,
  account_slice_from_response,
  optional_accounts_slice_from_response,
  account_from_response,
  multiple_accounts_from_response
)

fn response(value :: String, context :: String) -> RpcResponse!String do
  rpc_response("{\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"context\":#{context},\"value\":#{value}}}")
end

fn account(space :: String) -> String do
  "{\"data\":[\"AQID\",\"base64\"],\"executable\":false,\"lamports\":1,\"owner\":\"11111111111111111111111111111111\",\"rentEpoch\":18446744073709551615,\"space\":#{space}}"
end

fn proof() -> Bool!String do
  let address = pubkey("11111111111111111111111111111111")?
  let context = "{\"slot\":18446744073709551615}"
  let minimum = Some(U64.parse("18446744073709551615")?)
  let numeric_owner = String.replace(account("65616"),
    "\"owner\":\"11111111111111111111111111111111\"",
    "\"owner\":11111111111111111111111111111111")
  case account_slice_from_response(response(numeric_owner, context)?) do
    Ok(_) -> assert(false)
    Err(_) -> nil
  end
  let request = get_multiple_accounts_slice_request(7,
    [address, address],
    "finalized",
    0,
    2640,
    minimum)?
  let wire = rpc_request_json(request)
  assert(String.contains(wire, "\"method\":\"getMultipleAccounts\""))
  assert(String.contains(wire, "\"dataSlice\":{\"offset\":0,\"length\":2640}"))
  assert(String.contains(wire, "\"minContextSlot\":18446744073709551615"))
  for (offset, length) in [(-1, 1), (0, -1), (10485760, 1), (9223372036854775807, 1)] do
    case get_multiple_accounts_slice_request(7, [address], "finalized", offset, length, None) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  for addresses in [
    [],
    for _ in 0..101 do
      address
    end,
    [Pubkey { bytes: Bytes.empty() }]
  ] do
    case get_multiple_accounts_slice_request(7, addresses, "finalized", 0, 1, None) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  let single = account_slice_from_response(response(account("65616"), context)?)?
  assert(U64.to_string(single.slot) == "18446744073709551615")
  case single.account do
    None -> assert(false)
    Some(value) -> do
      assert(U64.to_string(value.space) == "65616")
      assert(Bytes.to_hex(value.info.data) == "010203")
    end
  end
  let absent = account_slice_from_response(response("null", context)?)?
  case absent.account do
    None -> nil
    Some(_) -> assert(false)
  end
  let batch = optional_accounts_slice_from_response(response("[null,#{account("65616")} ]",
    context)?)?
  assert(batch.slot == single.slot && List.length(batch.accounts) == 2)
  case List.get(batch.accounts, 0) do
    None -> nil
    Some(_) -> assert(false)
  end
  case List.get(batch.accounts, 1) do
    None -> assert(false)
    Some(value) -> assert(value.space == U64.parse("65616")?)
  end
  for invalid in [
    account("2"),
    account("\"65616\""),
    account("3.0"),
    account("-1"),
    account("null"),
    Json.encode_string(account("65616"))
  ] do
    case account_slice_from_response(response(invalid, context)?) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
    case optional_accounts_slice_from_response(response("[#{invalid}]", context)?) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  for bad_context in [Json.encode_string(context), "{\"slot\":\"42\"}", "null", "[]"] do
    case account_slice_from_response(response("null", bad_context)?) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
    case optional_accounts_slice_from_response(response("[]", bad_context)?) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
    case account_from_response(response(account("65616"), bad_context)?) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
    case multiple_accounts_from_response(response("[#{account("65616")} ]", bad_context)?) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  case account_from_response(response(Json.encode_string(account("65616")), context)?) do
    Ok(_) -> assert(false)
    Err(_) -> nil
  end
  case multiple_accounts_from_response(response(Json.encode_string("[#{account("65616")}]"),
    context)?) do
    Ok(_) -> assert(false)
    Err(_) -> nil
  end
  let too_many = "["
    <> String.join(for _ in 0..101 do
        "null"
      end,
      ",")
    <> "]"
  case optional_accounts_slice_from_response(response(too_many, context)?) do
    Ok(_) -> assert(false)
    Err(_) -> nil
  end
  Ok(true)
end

test("sliced account batches preserve one context, null positions and full space without JSON coercion") do
  case proof() do
    Err(error) -> do
      println(error)
      assert(false)
    end
    Ok(value) -> assert(value)
  end
end
