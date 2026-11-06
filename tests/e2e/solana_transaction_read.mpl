from Solana.Read import rpc_response
from Solana.TransactionRead import (
  signed_transaction,
  finalized_transaction_from_response,
  get_finalized_transaction_request
)
from Solana.Read import rpc_request_json

fn require(value :: Bool, label :: String) -> Result<(), String> do
  if value do
    Ok(nil)
  else
    Err(label)
  end
end

fn proof() -> Bool!String do
  let root = Json.parse(File.read(Env.get("FIXTURE", ""))?)?
  let fixtures = Json.object_get(root, "fixtures")?
  for index in 0..Json.array_length(fixtures)? do
    let fixture = Json.array_get(fixtures, index)?
    let wire = Bytes.from_hex(Json.as_string(Json.object_get(fixture, "wire_hex")?)?)?
    let signature = Bytes.from_hex(Json.as_string(Json.object_get(fixture, "signature_hex")?)?)?
    let decoded = signed_transaction(wire, signature)?
    require(Bytes.to_hex(decoded.message) == Json.as_string(Json.object_get(fixture,
          "message_hex")?)?,
      "message mismatch")?
    require(Bytes.to_hex(decoded.wire) == Bytes.to_hex(wire), "wire mismatch")?
    let keys = Json.object_get(fixture, "account_keys")?
    require(List.length(decoded.accounts) == Json.array_length(keys)?, "key count")?
    for key in 0..List.length(decoded.accounts) do
      require(Bytes.to_base58(List.get(decoded.accounts,
          key).pubkey.bytes) == Json.as_string(Json.array_get(keys, key)?)?,
        "key mismatch")?
    end
    if List.length(decoded.signatures) > 1 do
      let bad = Bytes.concat(Bytes.slice(wire, 0, 65)?,
        Bytes.concat(Bytes.from_hex(String.repeat("00", 64))?,
          Bytes.slice(wire, 129, Bytes.length(wire) - 129)?)?)?
      case signed_transaction(bad, signature) do
        Ok(_) -> return Err("invalid second signature accepted")
        Err(_) -> nil
      end
    end
    let response = rpc_response(Json.as_string(Json.object_get(fixture, "response")?)?)?
    let result = case finalized_transaction_from_response(response, signature, 41)? do
      None -> return Err("missing transaction")
      Some(value) -> value
    end
    require(U64.to_string(result.slot) == "18446744073709551615", "slot precision")?
    require(result.inner_instruction_count == 0, "inner")?
    let wrong = Bytes.from_hex(String.repeat("00", 64))?
    case signed_transaction(wire, wrong) do
      Ok(_) -> return Err("wrong signature accepted")
      Err(_) -> nil
    end
    case signed_transaction(Bytes.concat(wire, Bytes.from_hex("00")?)?, signature) do
      Ok(_) -> return Err("trailing bytes accepted")
      Err(_) -> nil
    end
    case finalized_transaction_from_response(response, signature, 42) do
      Ok(_) -> return Err("wrong id accepted")
      Err(_) -> nil
    end
    let request = rpc_request_json(get_finalized_transaction_request(41, signature)?)
    require(Json.get(request, "method") == "getTransaction", "method")?
  end
  Ok(true)
end

fn main() do
  case proof() do
    Ok(_) -> println("transaction-fixtures:ok")
    Err(error) -> println(error)
  end
end
