from Solana.Read import Pubkey, Hash, Signature, pubkey
from Solana.Tx import AccountMeta, Instruction, compile_legacy_message, serialize_legacy_message
from Solana.TransactionRead import signed_transaction
from Solana.TransactionWrite import serialize_signed_legacy_transaction

fn pair() -> SigningKeyPair!String do
  case Crypto.signing_generate() do
    Err(_) -> Err("test signing key failed")
    Ok(value)
  end
end

fn proof() -> Bool!String do
  let payer = pair()?
  let source = pair()?
  let instruction = Instruction {
    program_id: pubkey("11111111111111111111111111111111")?,
    accounts: [
      AccountMeta {
        pubkey: Pubkey { bytes: source.public_key.bytes },
        signer: true,
        writable: true
      },
      AccountMeta {
        pubkey: Pubkey { bytes: Bytes.from_hex(String.repeat("03", 32))? },
        signer: false,
        writable: true
      }
    ],
    data: Bytes.from_hex("020000000100000000000000")?
  }
  let message = compile_legacy_message(Pubkey { bytes: payer.public_key.bytes },
    Hash { bytes: Bytes.from_hex(String.repeat("04", 32))? },
    [instruction])?
  let bytes = serialize_legacy_message(message)?
  let first = case Crypto.sign(payer.private_key, bytes) do
    Err(_) -> return Err("test signing failed")
    Ok(value) -> value
  end
  let second = case Crypto.sign(source.private_key, bytes) do
    Err(_) -> return Err("test signing failed")
    Ok(value) -> value
  end
  let wire = serialize_signed_legacy_transaction(message, [first, second])?
  let decoded = signed_transaction(wire, first.bytes)?
  assert(decoded.version == -1 && decoded.header.num_required_signatures == 2)
  assert(Bytes.secure_equals(decoded.message, bytes))
  assert(Bytes.secure_equals(List.get(decoded.signatures, 1), second.bytes))
  for signatures in [
    [],
    [first],
    [first, second, second],
    [second, first],
    [first, Signature { bytes: Bytes.from_hex(String.repeat("00", 64))? }],
    [first, Signature { bytes: Bytes.from_hex(String.repeat("00", 63))? }]
  ] do
    assert(Result.is_err(serialize_signed_legacy_transaction(message, signatures)))
  end
  assert(Result.is_err(serialize_signed_legacy_transaction(%{message |
      recent_blockhash: Hash { bytes: Bytes.from_hex(String.repeat("05", 32))? }
    },
    [first, second])))
  assert(Result.is_err(serialize_signed_legacy_transaction(%{message |
      header: %{message.header | num_required_signatures: 0}
    },
    [])))
  let oversized = compile_legacy_message(Pubkey { bytes: payer.public_key.bytes },
    message.recent_blockhash,
    [%{instruction | data: Bytes.from_hex(String.repeat("01", 1200))?}])?
  assert(Result.is_err(serialize_signed_legacy_transaction(oversized, [first, second])))
  Ok(true)
end

test("signed legacy serialization authenticates every ordered signature and exact packet") do
  case proof() do
    Ok(value) -> assert(value)
    Err(error) -> do
      println(error)
      assert(false)
    end
  end
end
