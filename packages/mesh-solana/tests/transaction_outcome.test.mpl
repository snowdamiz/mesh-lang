from Solana.Read import rpc_response
from Solana.TransactionRead import (
  finalized_transaction_from_response,
  finalized_transaction_outcome_from_response
)

fn proof() -> Bool!String do
  let raw = "{\"jsonrpc\":\"2.0\",\"id\":41,\"result\":{\"slot\":18446744073709551615,\"blockTime\":1001,\"version\":\"legacy\",\"transaction\":[\"AdPpy6ii2WRosK6Vkoz3crwAX3cSXT0vdVuPtyOqxQEVr/9beYbvoO4KZO9H3IKWiY7Wsl/EV7VtVKYvEqz+2AkBAAIEiojj3XQJ8ZX9UtstPLpdcspnCb8dlBIb83SIAbQPb1wCAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAECAwABAwwCAAAAQEtMAAAAAAA=\",\"base64\"],\"meta\":{\"err\":null,\"fee\":5000,\"loadedAddresses\":{\"writable\":[],\"readonly\":[]},\"innerInstructions\":[],\"preBalances\":[1000000000000000000,0,0,0],\"postBalances\":[999999999994995000,5000000,0,0],\"preTokenBalances\":[],\"postTokenBalances\":[]}}}"
  let signature = Bytes.from_hex("d3e9cba8a2d96468b0ae95928cf772bc005f77125d3d2f755b8fb723aac50115afff5b7986efa0ee0a64ef47dc8296898ed6b25fc457b56d54a62f12acfed809")?
  let success = case finalized_transaction_outcome_from_response(rpc_response(raw)?,
    signature,
    41)? do
    None -> return Err("lost successful transaction")
    Some(value) -> value
  end
  assert(success.execution_error == None)
  assert(success.transaction.slot == U64.parse("18446744073709551615")?)
  for error in [
    "{\"InstructionError\":[0,{\"Custom\":1}]}",
    "{\"InstructionError\":[0,\"InvalidArgument\"]}",
    "\"AccountInUse\""
  ] do
    let failed = String.replace(raw, "\"err\":null", "\"err\":" <> error)
    assert(Result.is_err(finalized_transaction_from_response(rpc_response(failed)?, signature, 41)))
    let outcome = case finalized_transaction_outcome_from_response(rpc_response(failed)?,
      signature,
      41)? do
      None -> return Err("lost failed transaction")
      Some(value) -> value
    end
    assert(outcome.execution_error == Some(error))
    assert(Bytes.secure_equals(outcome.transaction.signed.wire, success.transaction.signed.wire))
    assert(outcome.transaction.fee == success.transaction.fee)
  end
  for error in [
    "true",
    "1",
    "[]",
    "{}",
    "\"\"",
    "\"not a variant\"",
    "{\"InstructionError\":[1,\"InvalidArgument\"]}",
    "{\"InstructionError\":[-1,\"InvalidArgument\"]}",
    "{\"InstructionError\":[0.0,\"InvalidArgument\"]}",
    "{\"InstructionError\":[0,{\"Custom\":4294967296}]}",
    "{\"InstructionError\":[0,{\"Custom\":\"1\"}]}",
    "{\"InstructionError\":[0,\"InvalidArgument\"],\"extra\":1}"
  ] do
    assert(Result.is_err(finalized_transaction_outcome_from_response(rpc_response(String.replace(raw,
        "\"err\":null",
        "\"err\":" <> error))?,
      signature,
      41)))
  end
  assert(Result.is_err(finalized_transaction_outcome_from_response(rpc_response(raw)?,
    signature,
    42)))
  assert(Result.is_err(finalized_transaction_outcome_from_response(rpc_response(raw)?,
    Bytes.from_hex(String.repeat("00", 64))?,
    41)))
  assert(finalized_transaction_outcome_from_response(rpc_response("{\"jsonrpc\":\"2.0\",\"id\":41,\"result\":null}")?,
    signature,
    41)? == None)
  Ok(true)
end

test("finalized outcomes authenticate failed packets without accepting malformed execution status") do
  case proof() do
    Ok(value) -> assert(value)
    Err(error) -> do
      println(error)
      assert(false)
    end
  end
end
