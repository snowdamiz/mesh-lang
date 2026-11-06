from Solana.Read import rpc_response, latest_blockhash_from_response
from Solana.TransactionSend import sent_signature_from_response

fn proof() -> Bool!String do
  let hash = Bytes.from_hex(String.repeat("01", 32))?
  let raw = "{\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"context\":{\"slot\":9007199254740993},\"value\":{\"blockhash\":"
    <> Json.encode_string(Bytes.to_base58(hash))
    <> ",\"lastValidBlockHeight\":18446744073709551615}}}"
  let value = latest_blockhash_from_response(rpc_response(raw)?)?
  assert(value.context_slot == U64.parse("9007199254740993")?)
  assert(value.last_valid_block_height == U64.parse("18446744073709551615")?)
  assert(Bytes.secure_equals(value.blockhash.bytes, hash))
  for bad in ["\"3\"", "3.0", "-1", "18446744073709551616", "true"] do
    assert(Result.is_err(latest_blockhash_from_response(rpc_response(String.replace(raw,
      "9007199254740993",
      bad))?)))
  end
  let signature = Bytes.from_hex(String.repeat("02", 64))?
  let response = rpc_response("{\"jsonrpc\":\"2.0\",\"id\":7,\"result\":"
    <> Json.encode_string(Bytes.to_base58(signature))
    <> "}")?
  assert(Bytes.secure_equals(sent_signature_from_response(response, signature)?, signature))
  assert(Result.is_err(sent_signature_from_response(response,
    Bytes.from_hex(String.repeat("03", 64))?)))
  Ok(true)
end

test("bounded payment RPC decoders preserve u64 and bind exact signed signature") do
  case proof() do
    Ok(value) -> assert(value)
    Err(error) -> do
      println(error)
      assert(false)
    end
  end
end
