from Solana.Read import Signature
from Solana.Tx import LegacyMessage, serialize_legacy_message
from Solana.TransactionRead import signed_transaction

fn signature_bytes(signatures :: List<Signature>, index :: Int, output :: Bytes) -> Bytes!String do
  if index >= List.length(signatures) do
    return Ok(output)
  end
  let signature = List.get(signatures, index).bytes
  if Bytes.length(signature) != 64 do
    return Err("SOLANA_WRITE: invalid signature length")
  end
  signature_bytes(signatures, index + 1, Bytes.concat(output, signature)?)
end

# This bounded packet shares the authenticated reader's signature/account/
# instruction limits. It accepts public signatures, never secret-key lists.
pub fn serialize_signed_legacy_transaction(message :: LegacyMessage,
  signatures :: List<Signature>) -> Bytes!String do
  let count = List.length(signatures)
  if count < 1 || count > 12 || count != message.header.num_required_signatures do
    return Err("SOLANA_WRITE: invalid signature count")
  end
  let prefix = signature_bytes(signatures, 0, Bytes.write_uint_le(Int.to_string(count), 1)?)?
  let wire = Bytes.concat(prefix, serialize_legacy_message(message)?)?
  # Reuse the canonical parser to reject duplicate keys, invalid roles/indices
  # and signatures that do not authenticate every byte of this exact message.
  let authenticated = signed_transaction(wire, List.head(signatures).bytes)?
  if authenticated.version != -1 do
    return Err("SOLANA_WRITE: expected legacy message")
  end
  Ok(authenticated.wire)
end
