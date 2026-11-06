from Solana.Read import Pubkey, Hash, AccountInfo, pubkey, pubkey_equal

pub struct SlotHashEntry do
  slot :: U64
  hash :: Hash
end

pub struct SlotHashes do
  entries :: List<SlotHashEntry>
end

pub fn slot_hashes_sysvar_address() -> Pubkey!String do
  pubkey("SysvarS1otHashes111111111111111111111111111")
end

fn validate(value :: SlotHashes) -> Result<(), String> do
  if List.length(value.entries) > 512 do
    return Err("SOLANA_SLOT_HASHES: too many entries")
  end
  for index in 0..List.length(value.entries) do
    let entry = List.get(value.entries, index)
    if Bytes.length(entry.hash.bytes) != 32
      || (index > 0 && U64.compare(List.get(value.entries, index - 1).slot, entry.slot) <= 0) do
      return Err("SOLANA_SLOT_HASHES: entries must be distinct and descending")
    end
  end
  Ok(nil)
end

# Accept SDK serialization or the fixed sysvar allocation. Like the SDK, only
# the counted entries are interpreted; unused allocation bytes are not history.
# Callers bind the requested address and authenticated RPC context separately.
pub fn slot_hashes_sysvar(account :: AccountInfo) -> SlotHashes!String do
  let bytes = account.data
  if account.executable
    || account.lamports == U64.parse("0")?
    || !pubkey_equal(account.owner, pubkey("Sysvar1111111111111111111111111111111111111")?)
    || Bytes.length(bytes) < 8
    || Bytes.length(bytes) > 20488 do
    return Err("SOLANA_SLOT_HASHES: invalid sysvar account")
  end
  let wide_count = U64.parse(Bytes.read_uint_le(bytes, 0, 8)?)?
  if U64.compare(wide_count, U64.parse("512")?) > 0 do
    return Err("SOLANA_SLOT_HASHES: too many entries")
  end
  let count = U64.to_int(wide_count)?
  if Bytes.length(bytes) != 8 + count * 40 && Bytes.length(bytes) != 20488 do
    return Err("SOLANA_SLOT_HASHES: invalid serialized length")
  end
  let value = SlotHashes {
    entries: for index in 0..count do
      SlotHashEntry {
        slot: U64.parse(Bytes.read_uint_le(bytes, 8 + index * 40, 8)?)?,
        hash: Hash { bytes: Bytes.slice(bytes, 16 + index * 40, 32)? }
      }
    end
  }
  validate(value)?
  Ok(value)
end

pub fn slot_hashes_position(value :: SlotHashes, slot :: U64) -> Option<Int>!String do
  validate(value)?
  for index in 0..List.length(value.entries) do
    if List.get(value.entries, index).slot == slot do
      return Ok(Some(index))
    end
  end
  Ok(None)
end
