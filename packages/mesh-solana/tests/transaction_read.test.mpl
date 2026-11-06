from Solana.TransactionRead import signed_transaction

# Minimal framing must fail before interpreting an account or signature.
fn proof() -> Bool!String do
  for data in [
    Bytes.empty(),
    Bytes.from_hex("0100")?,
    Bytes.from_hex(String.repeat("00", 1233))?
  ] do
    case signed_transaction(data, Bytes.from_hex(String.repeat("00", 64))?) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  Ok(true)
end

test("signed transaction reader rejects empty, truncated and oversized wire") do
  case proof() do
    Ok(value) -> assert(value)
    Err(error) -> do
      println(error)
      assert(false)
    end
  end
end
