from Solana.Read import Pubkey, Hash, AccountInfo, pubkey
from Solana.SlotHashes import (
  SlotHashEntry,
  SlotHashes,
  slot_hashes_sysvar_address,
  slot_hashes_sysvar,
  slot_hashes_position
)
from Solana.LookupTable import LookupTableAccount, LookupTableStatus, lookup_table_status

fn raw(slots :: List<String>) -> Bytes!String do
  List.reduce(slots,
    Bytes.write_uint_le(Int.to_string(List.length(slots)), 8),
    fn(result, slot) do Bytes.concat(Bytes.concat(result?, Bytes.write_uint_le(slot, 8)?)?,
      Bytes.from_hex(String.repeat("11", 32))?) end)
end

fn proof() -> Bool!String do
  assert(Bytes.to_base58(slot_hashes_sysvar_address()?.bytes) == "SysvarS1otHashes111111111111111111111111111")
  let account = AccountInfo {
    owner: pubkey("Sysvar1111111111111111111111111111111111111")?,
    data: raw(["9007199254740994", "9007199254740993", "9007199254740991"])?,
    lamports: U64.parse("1")?,
    executable: false,
    rent_epoch: U64.parse("0")?
  }
  let hashes = slot_hashes_sysvar(account)?
  assert(List.length(hashes.entries) == 3)
  assert(U64.to_string(List.head(hashes.entries).slot) == "9007199254740994")
  assert(Bytes.to_hex(List.head(hashes.entries).hash.bytes) == String.repeat("11", 32))
  assert(slot_hashes_position(hashes, U64.parse("9007199254740993")?)? == Some(1))
  assert(slot_hashes_position(hashes, U64.parse("9007199254740992")?)? == None)
  let padded = %{account |
    data: Bytes.concat(account.data,
      Bytes.from_hex(String.repeat("ab", 20488 - Bytes.length(account.data)))?)?
  }
  assert(slot_hashes_sysvar(padded)? == hashes)
  let empty = slot_hashes_sysvar(%{account | data: raw([])?})?
  assert(List.length(empty.entries) == 0)
  let table = LookupTableAccount {
    authority: None,
    deactivation_slot: U64.parse("9007199254740993")?,
    last_extended_slot: U64.parse("0")?,
    last_extended_start_index: 0,
    addresses: []
  }
  assert(lookup_table_status(table, U64.parse("9007199254740994")?, hashes)? == Deactivating(511))
  assert(lookup_table_status(table, table.deactivation_slot, empty)? == Deactivating(513))
  assert(lookup_table_status(table, U64.parse("9007199254740994")?, empty)? == Deactivated)
  assert(lookup_table_status(%{table | deactivation_slot: U64.parse("18446744073709551615")?},
    U64.parse("9007199254740994")?,
    empty)? == Activated)
  let full = slot_hashes_sysvar(%{account |
    data: raw(for n in 0..512 do
      Int.to_string(1000 - n)
    end)?
  })?
  assert(lookup_table_status(%{table | deactivation_slot: U64.parse("489")?},
    U64.parse("1001")?,
    full)? == Deactivating(1))
  assert(lookup_table_status(%{table | deactivation_slot: U64.parse("488")?},
    U64.parse("1001")?,
    full)? == Deactivated)
  for bad in [
    %{account | executable: true},
    %{account | owner: pubkey("11111111111111111111111111111111")?},
    %{account | lamports: U64.parse("0")?},
    %{account | data: Bytes.from_hex(String.repeat("00", 7))?},
    %{account | data: Bytes.concat(account.data, Bytes.from_hex("00")?)?},
    %{account | data: raw(["1", "2"])?},
    %{account | data: raw(["2", "2"])?},
    %{account |
      data: raw(for n in 0..513 do
        Int.to_string(1000 - n)
      end)?
    },
    %{account | data: Bytes.from_hex("ffffffffffffffff")?}
  ] do
    case slot_hashes_sysvar(bad) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  for current in ["9007199254740992", "9007199254740993"] do
    case lookup_table_status(table, U64.parse(current)?, hashes) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  case lookup_table_status(%{table | last_extended_slot: U64.parse("9007199254740995")?},
    U64.parse("9007199254740994")?,
    empty) do
    Ok(_) -> assert(false)
    Err(_) -> nil
  end
  Ok(true)
end

test("SlotHashes decoding and lookup cooldown preserve exact slots and reject impossible bank history") do
  case proof() do
    Ok(value) -> assert(value)
    Err(error) -> do
      println(error)
      assert(false)
    end
  end
end
