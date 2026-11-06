from Solana.Read import Pubkey, AccountInfo, pubkey, pubkey_equal
from Solana.Address import ProgramDerivedAddress, find_program_address
from Solana.Tx import Instruction, AccountMeta
from Solana.SlotHashes import SlotHashes, SlotHashEntry, slot_hashes_position

pub type LookupTableStatus do
  Activated
  Deactivating(Int)
  Deactivated
end

pub struct LookupTableCreation do
  address :: Pubkey
  instruction :: Instruction
end

pub struct LookupTableAccount do
  authority :: Option<Pubkey>
  deactivation_slot :: U64
  last_extended_slot :: U64
  last_extended_start_index :: Int
  addresses :: List<Pubkey>
end

# Matches LookupTableMeta::status for a valid bank. A future table/history
# coordinate is an error, never evidence that the deactivation slot expired.
# Authority and requested-address authentication remain the caller's job.
pub fn lookup_table_status(table :: LookupTableAccount,
  current_slot :: U64,
  hashes :: SlotHashes) -> LookupTableStatus!String do
  let position = slot_hashes_position(hashes, table.deactivation_slot)?
  let active = table.deactivation_slot == U64.parse("18446744073709551615")?
  if U64.compare(table.last_extended_slot, current_slot) > 0
    || (!active && U64.compare(table.deactivation_slot, current_slot) > 0) do
    return Err("SOLANA_LOOKUP: table metadata is from a future bank")
  end
  for entry in hashes.entries do
    if U64.compare(entry.slot, current_slot) > 0 do
      return Err("SOLANA_LOOKUP: slot history is from a future bank")
    end
  end
  if active do
    Ok(Activated)
  else if table.deactivation_slot == current_slot do
    Ok(Deactivating(513))
  else
    case position do
      Some(index) -> Ok(Deactivating(512 - index))
      None -> Ok(Deactivated)
    end
  end
end

pub fn lookup_table_program() -> Pubkey!String do
  pubkey("AddressLookupTab1e1111111111111111111111111")
end

fn meta(key :: Pubkey, signer :: Bool, writable :: Bool) -> AccountMeta!String do
  if Bytes.length(key.bytes) != 32 do
    return Err("SOLANA_LOOKUP: instruction key must be 32 bytes")
  end
  Ok(AccountMeta { pubkey: key, signer: signer, writable: writable })
end

fn cat(values :: List<Bytes>) -> Bytes!String do
  if List.length(values) == 0 do
    Ok(Bytes.empty())
  else
    Bytes.concat(List.head(values), cat(List.tail(values))?)
  end
end

# Matches the official create instruction: authority need not sign creation;
# later extension, deactivation and closure require its signature. The caller
# must obtain a recent slot and authenticate the derived account separately.
pub fn create_lookup_table_instruction(authority :: Pubkey,
  payer :: Pubkey,
  recent_slot :: U64) -> LookupTableCreation!String do
  let authority_meta = meta(authority, false, false)?
  let payer_meta = meta(payer, true, true)?
  let program = lookup_table_program()?
  let slot = Bytes.write_uint_le(U64.to_string(recent_slot), 8)?
  let derived = find_program_address(program, [authority.bytes, slot])?
  Ok(LookupTableCreation {
    address: derived.address,
    instruction: Instruction {
      program_id: program,
      accounts: [
        meta(derived.address, false, true)?,
        authority_meta,
        payer_meta,
        meta(pubkey("11111111111111111111111111111111")?, false, false)?
      ],
      data: cat([
        Bytes.write_uint_le("0", 4)?,
        slot,
        Bytes.write_uint_le(Int.to_string(derived.bump), 1)?
      ])?
    }
  })
end

# The 256-entry protocol bound is not a packet-size promise. Writers must split
# extensions into transactions that fit their actual signer/account overhead.
# Duplicated addresses are representable, as in the on-chain program.
pub fn extend_lookup_table_instruction(table :: Pubkey,
  authority :: Pubkey,
  payer :: Option<Pubkey>,
  addresses :: List<Pubkey>) -> Instruction!String do
  if List.length(addresses) < 1 || List.length(addresses) > 256 do
    return Err("SOLANA_LOOKUP: extension must contain 1 to 256 addresses")
  end
  let accounts = [meta(table, false, true)?, meta(authority, true, false)?]
  let accounts = case payer do
    None -> accounts
    Some(value) -> accounts
      ++ [
        meta(value, true, true)?,
        meta(pubkey("11111111111111111111111111111111")?, false, false)?
      ]
  end
  let raw = for address in addresses do
    meta(address, false, false)?
    address.bytes
  end
  Ok(Instruction {
    program_id: lookup_table_program()?,
    accounts: accounts,
    data: cat([
      Bytes.write_uint_le("2", 4)?,
      Bytes.write_uint_le(Int.to_string(List.length(addresses)), 8)?
    ]
      ++ raw)?
  })
end

pub fn deactivate_lookup_table_instruction(table :: Pubkey,
  authority :: Pubkey) -> Instruction!String do
  Ok(Instruction {
    program_id: lookup_table_program()?,
    accounts: [meta(table, false, true)?, meta(authority, true, false)?],
    data: Bytes.write_uint_le("3", 4)?
  })
end

# Closure only succeeds after deactivation has aged out of SlotHashes; these
# bytes do not authorize early rent recovery or prove an account is closable.
pub fn close_lookup_table_instruction(table :: Pubkey,
  authority :: Pubkey,
  recipient :: Pubkey) -> Instruction!String do
  Ok(Instruction {
    program_id: lookup_table_program()?,
    accounts: [
      meta(table, false, true)?,
      meta(authority, true, false)?,
      meta(recipient, false, true)?
    ],
    data: Bytes.write_uint_le("4", 4)?
  })
end

fn zero(bytes :: Bytes) -> Bool do
  Bytes.to_hex(bytes) == String.repeat("00", Bytes.length(bytes))
end

# Decode the initialized 56-byte metadata and 0..256 addresses. Callers bind
# the requested address, authority and finalized context. In particular, newly
# appended addresses cannot be used in the same slot as the extension; a
# deactivating table's lifetime depends on SlotHashes, not elapsed wall time.
pub fn lookup_table_account(account :: AccountInfo) -> LookupTableAccount!String do
  let raw = account.data
  let length = Bytes.length(raw)
  if account.executable
    || !pubkey_equal(account.owner, lookup_table_program()?)
    || account.lamports == U64.parse("0")?
    || length < 56
    || length > 8248
    || (length - 56) % 32 != 0 do
    return Err("SOLANA_LOOKUP: invalid table account")
  end
  let count = (length - 56) / 32
  let start = Bytes.get(raw, 20)?
  if Bytes.read_uint_le(raw, 0, 4)? != "1" || start > count do
    return Err("SOLANA_LOOKUP: invalid table metadata")
  end
  let authority = case Bytes.get(raw, 21)? do
    0 -> do
      if !zero(Bytes.slice(raw, 22, 34)?) do
        return Err("SOLANA_LOOKUP: invalid frozen metadata")
      end
      None
    end
    1 -> do
      if !zero(Bytes.slice(raw, 54, 2)?) do
        return Err("SOLANA_LOOKUP: invalid metadata padding")
      end
      Some(Pubkey { bytes: Bytes.slice(raw, 22, 32)? })
    end
    _ -> return Err("SOLANA_LOOKUP: invalid authority option")
  end
  Ok(LookupTableAccount {
    authority: authority,
    deactivation_slot: U64.parse(Bytes.read_uint_le(raw, 4, 8)?)?,
    last_extended_slot: U64.parse(Bytes.read_uint_le(raw, 12, 8)?)?,
    last_extended_start_index: start,
    addresses: for index in 0..count do
      Pubkey { bytes: Bytes.slice(raw, 56 + index * 32, 32)? }
    end
  })
end
