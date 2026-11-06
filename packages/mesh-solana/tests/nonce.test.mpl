from Solana.Read import Pubkey, Hash, AccountInfo, pubkey, pubkey_equal
from Solana.Tx import Instruction, compute_unit_limit_instruction, serialize_legacy_message
from Solana.Nonce import (
  nonce_account,
  advance_nonce_account_instruction,
  compile_durable_nonce_message
)

fn fixture() -> AccountInfo!String do
  Ok(AccountInfo {
    owner: pubkey("11111111111111111111111111111111")?,
    executable: false,
    lamports: U64.parse("1000000")?,
    rent_epoch: U64.parse("0")?,
    data: Bytes.from_hex("0100000001000000"
      <> String.repeat("11", 32)
      <> String.repeat("22", 32)
      <> "ffffffffffffffff")?
  })
end

fn proof() -> Bool!String do
  let account = fixture()?
  let state = nonce_account(account)?
  assert(Bytes.to_hex(state.authority.bytes) == String.repeat("11", 32))
  assert(Bytes.to_hex(state.nonce.bytes) == String.repeat("22", 32))
  assert(state.lamports_per_signature == U64.parse("18446744073709551615")?)
  for data in [
    Bytes.from_hex("0000000001000000"
      <> String.repeat("11", 32)
      <> String.repeat("22", 32)
      <> "ffffffffffffffff")?,
    Bytes.from_hex("0200000001000000"
      <> String.repeat("11", 32)
      <> String.repeat("22", 32)
      <> "ffffffffffffffff")?,
    Bytes.from_hex("0100000000000000" <> String.repeat("00", 72))?,
    Bytes.from_hex("0100000002000000" <> String.repeat("00", 72))?,
    Bytes.slice(account.data, 0, 79)?,
    Bytes.concat(account.data, Bytes.from_hex("00")?)?
  ] do
    assert(Result.is_err(nonce_account(%{account | data: data})))
  end
  assert(Result.is_err(nonce_account(%{account |
    owner: pubkey("Vote111111111111111111111111111111111111111")?
  })))
  assert(Result.is_err(nonce_account(%{account | executable: true})))
  assert(Result.is_err(nonce_account(%{account | lamports: U64.parse("0")?})))
  let address = Pubkey { bytes: Bytes.from_hex(String.repeat("33", 32))? }
  let payer = Pubkey { bytes: Bytes.from_hex(String.repeat("44", 32))? }
  let advance = advance_nonce_account_instruction(address, state.authority)?
  assert(pubkey_equal(advance.program_id, account.owner)
    && Bytes.to_hex(advance.data) == "04000000")
  assert(List.length(advance.accounts) == 3)
  let source = List.get(advance.accounts, 0)
  let sysvar = List.get(advance.accounts, 1)
  let authority = List.get(advance.accounts, 2)
  assert(pubkey_equal(source.pubkey, address) && source.writable && !source.signer)
  assert(Bytes.to_base58(sysvar.pubkey.bytes) == "SysvarRecentB1ockHashes11111111111111111111"
    && !sysvar.writable
    && !sysvar.signer)
  assert(pubkey_equal(authority.pubkey, state.authority) && authority.signer && !authority.writable)
  let limit = compute_unit_limit_instruction(100000)?
  let message = compile_durable_nonce_message(payer, address, state, [limit])?
  assert(message.header.num_required_signatures == 2 && List.length(message.instructions) == 2)
  assert(Bytes.secure_equals(message.recent_blockhash.bytes, state.nonce.bytes))
  assert(Bytes.to_hex(List.get(message.instructions, 0).data) == "04000000")
  assert(Bytes.secure_equals(List.get(message.instructions, 1).data, limit.data))
  assert(Bytes.length(serialize_legacy_message(message)?) > 0)
  assert(Result.is_err(compile_durable_nonce_message(payer, address, state, [advance])))
  assert(Result.is_err(advance_nonce_account_instruction(Pubkey { bytes: Bytes.empty() },
    state.authority)))
  assert(Result.is_err(compile_durable_nonce_message(payer,
    address,
    %{state | nonce: Hash { bytes: Bytes.empty() }},
    [])))
  # Advance-only packets can deliberately retire a reserved nonce, but that
  # still cannot prove whether another packet using it previously transferred.
  assert(List.length(compile_durable_nonce_message(payer, address, state, [])?.instructions) == 1)
  Ok(true)
end

test("durable nonce uses exact current System state and an authenticated first instruction") do
  case proof() do
    Ok(value) -> assert(value)
    Err(error) -> do
      println(error)
      assert(false)
    end
  end
end
