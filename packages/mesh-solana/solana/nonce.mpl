from Solana.Read import Pubkey, Hash, AccountInfo, pubkey, pubkey_equal
from Solana.Tx import AccountMeta, Instruction, LegacyMessage, compile_legacy_message

pub struct NonceAccountState do
  authority :: Pubkey
  nonce :: Hash
  lamports_per_signature :: U64
end

# Only the current, domain-separated initialized nonce layout is usable for a
# durable transaction. Legacy and uninitialized allocations confer no lifetime.
pub fn nonce_account(account :: AccountInfo) -> NonceAccountState!String do
  if account.executable
    || account.lamports == U64.parse("0")?
    || !pubkey_equal(account.owner, pubkey("11111111111111111111111111111111")?)
    || Bytes.length(account.data) != 80 do
    return Err("SOLANA_NONCE: invalid nonce account")
  end
  if Bytes.read_uint_le(account.data, 0, 4)? != "1"
    || Bytes.read_uint_le(account.data, 4, 4)? != "1" do
    return Err("SOLANA_NONCE: initialized current nonce required")
  end
  Ok(NonceAccountState {
    authority: Pubkey { bytes: Bytes.slice(account.data, 8, 32)? },
    nonce: Hash { bytes: Bytes.slice(account.data, 40, 32)? },
    lamports_per_signature: U64.parse(Bytes.read_uint_le(account.data, 72, 8)?)?
  })
end

pub fn advance_nonce_account_instruction(address :: Pubkey,
  authority :: Pubkey) -> Instruction!String do
  if Bytes.length(address.bytes) != 32 || Bytes.length(authority.bytes) != 32 do
    return Err("SOLANA_NONCE: invalid nonce keys")
  end
  Ok(Instruction {
    program_id: pubkey("11111111111111111111111111111111")?,
    accounts: [
      AccountMeta { pubkey: address, signer: false, writable: true },
      AccountMeta {
        pubkey: pubkey("SysvarRecentB1ockHashes11111111111111111111")?,
        signer: false,
        writable: false
      },
      AccountMeta { pubkey: authority, signer: true, writable: false }
    ],
    data: Bytes.from_hex("04000000")?
  })
end

# The nonce advance precedes every caller instruction, including compute budget
# instructions. It is a lifetime marker, not evidence of transfer success.
pub fn compile_durable_nonce_message(payer :: Pubkey,
  address :: Pubkey,
  nonce :: NonceAccountState,
  instructions :: List<Instruction>) -> LegacyMessage!String do
  if Bytes.length(payer.bytes) != 32 || Bytes.length(nonce.nonce.bytes) != 32 do
    return Err("SOLANA_NONCE: invalid nonce message")
  end
  let advance = advance_nonce_account_instruction(address, nonce.authority)?
  for instruction in instructions do
    if pubkey_equal(instruction.program_id, advance.program_id)
      && Bytes.length(instruction.data) >= 4
      && Bytes.read_uint_le(instruction.data, 0, 4)? == "4" do
      return Err("SOLANA_NONCE: duplicate nonce advance")
    end
  end
  compile_legacy_message(payer, nonce.nonce, [advance] ++ instructions)
end
