from Solana.Read import RpcRequest, RpcResponse, rpc_request
from Solana.TransactionRead import signed_transaction

fn result(response :: RpcResponse) -> Json!String do
  if !response.ok do
    return Err("SOLANA_SEND: RPC rejected request")
  end
  Json.parse(response.result_json)
end

# Submission is not confirmation. Bind the response to the exact already
# authenticated signed packet; never accept a server-selected replacement.
pub fn sent_signature_from_response(response :: RpcResponse, expected :: Bytes) -> Bytes!String do
  let text = Json.as_string(result(response)?)?
  let signature = Bytes.from_base58(text)?
  if Bytes.length(expected) != 64
    || Bytes.length(signature) != 64
    || Bytes.to_base58(signature) != text
    || !Bytes.secure_equals(signature, expected) do
    return Err("SOLANA_SEND: transaction signature mismatch")
  end
  Ok(signature)
end

pub fn send_transaction_request(id :: Int,
  wire :: Bytes,
  expected :: Bytes,
  minimum_context_slot :: U64) -> RpcRequest!String do
  signed_transaction(wire, expected)?
  rpc_request(id,
    "sendTransaction",
    "["
      <> Json.encode_string(Bytes.to_base64(wire))
      <> ",{\"encoding\":\"base64\",\"skipPreflight\":false,\"preflightCommitment\":\"confirmed\",\"maxRetries\":0,\"minContextSlot\":"
      <> U64.to_string(minimum_context_slot)
      <> "}]")
end
