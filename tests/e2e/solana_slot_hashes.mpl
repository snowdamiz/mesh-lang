from Solana.Read import Pubkey, Hash, AccountInfo, pubkey
from Solana.SlotHashes import SlotHashEntry, SlotHashes, slot_hashes_sysvar
from Solana.LookupTable import (
  LookupTableAccount,
  LookupTableStatus,
  lookup_table_account,
  lookup_table_status
)

fn require(value :: Bool) do
  if !value do
    panic("SlotHashes SDK mismatch")
  end
end

fn text(value :: Json, name :: String) -> String!String do
  Json.as_string(Json.object_get(value, name)?)
end

fn proof() -> Result<(), String> do
  let cases = Json.parse(File.read(Env.get("MESH_SLOT_HASHES_FIXTURE", ""))?)?
  for index in 0..Json.array_length(cases)? do
    let item = Json.array_get(cases, index)?
    let base = AccountInfo {
      owner: pubkey("Sysvar1111111111111111111111111111111111111")?,
      data: Bytes.empty(),
      lamports: U64.parse("1")?,
      executable: false,
      rent_epoch: U64.parse("0")?
    }
    let table = lookup_table_account(%{base |
      owner: pubkey("AddressLookupTab1e1111111111111111111111111")?,
      data: Bytes.from_hex(text(item, "table")?)?
    })?
    for name in ["compact", "padded"] do
      let hashes = slot_hashes_sysvar(%{base | data: Bytes.from_hex(text(item, name)?)?})?
      let expected = Json.object_get(item, "entries")?
      require(List.length(hashes.entries) == Json.array_length(expected)?)
      for row in 0..List.length(hashes.entries) do
        let actual = List.get(hashes.entries, row)
        let want = Json.array_get(expected, row)?
        require(U64.to_string(actual.slot) == text(want, "slot")?)
        require(Bytes.to_hex(actual.hash.bytes) == text(want, "hash")?)
      end
      let status = lookup_table_status(table, U64.parse(text(item, "current")?)?, hashes)?
      case status do
        Activated -> require(text(item, "status")? == "active")
        Deactivated -> require(text(item, "status")? == "closed")
        Deactivating(remaining) -> do
          require(text(item, "status")? == "cooling")
          require(remaining == Json.as_int(Json.object_get(item, "remaining")?)?)
        end
      end
    end
  end
  Ok(nil)
end

fn main() do
  case proof() do
    Ok(_) -> println("solana-slot-hashes:ok")
    Err(error) -> panic(error)
  end
end
