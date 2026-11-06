from Solana.Read import Pubkey, AccountInfo, pubkey, pubkey_string
from Solana.Tx import Instruction, AccountMeta
from Solana.LookupTable import (
  LookupTableCreation,
  LookupTableAccount,
  create_lookup_table_instruction,
  extend_lookup_table_instruction,
  deactivate_lookup_table_instruction,
  close_lookup_table_instruction,
  lookup_table_account
)

fn require(value :: Bool) do
  if !value do
    panic("Solana lookup table SDK mismatch")
  end
end

fn text(value :: Json, field :: String) -> String!String do
  Json.as_string(Json.object_get(value, field)?)
end

fn instruction(actual :: Instruction, expected :: Json) -> Result<(), String> do
  require(pubkey_string(actual.program_id) == text(expected, "program")?)
  require(Bytes.to_base64(actual.data) == text(expected, "data")?)
  let accounts = Json.object_get(expected, "accounts")?
  require(List.length(actual.accounts) == Json.array_length(accounts)?)
  for index in 0..List.length(actual.accounts) do
    let meta = List.get(actual.accounts, index)
    let item = Json.array_get(accounts, index)?
    require(pubkey_string(meta.pubkey) == text(item, "address")?)
    require(meta.signer == Json.as_bool(Json.object_get(item, "signer")?)?)
    require(meta.writable == Json.as_bool(Json.object_get(item, "writable")?)?)
  end
  Ok(nil)
end

fn proof() -> Result<(), String> do
  let cases = Json.parse(File.read(Env.get("MESH_SOLANA_LOOKUP_FIXTURE", ""))?)?
  for index in 0..Json.array_length(cases)? do
    let item = Json.array_get(cases, index)?
    let authority = pubkey(text(item, "authority")?)?
    let payer = pubkey(text(item, "payer")?)?
    let created = create_lookup_table_instruction(authority,
      payer,
      U64.parse(text(item, "slot")?)?)?
    require(pubkey_string(created.address) == text(item, "table")?)
    instruction(created.instruction, Json.object_get(item, "create")?)?
    let keys = Json.object_get(item, "addresses")?
    let addresses = for n in 0..Json.array_length(keys)? do
      pubkey(Json.as_string(Json.array_get(keys, n)?)?)?
    end
    instruction(extend_lookup_table_instruction(created.address,
        authority,
        Some(payer),
        addresses)?,
      Json.object_get(item, "extend")?)?
    instruction(extend_lookup_table_instruction(created.address, authority, None, addresses)?,
      Json.object_get(item, "extend_no_payer")?)?
    instruction(deactivate_lookup_table_instruction(created.address, authority)?,
      Json.object_get(item, "deactivate")?)?
    instruction(close_lookup_table_instruction(created.address, authority, payer)?,
      Json.object_get(item, "close")?)?
    for field in ["account", "frozen_account"] do
      let decoded = lookup_table_account(AccountInfo {
        data: Bytes.from_hex(text(item, field)?)?,
        owner: created.instruction.program_id,
        lamports: U64.parse("1")?,
        executable: false,
        rent_epoch: U64.parse("0")?
      })?
      require(decoded.addresses == addresses)
      require(U64.to_string(decoded.last_extended_slot) == text(item, "slot")?)
      require(U64.to_string(decoded.deactivation_slot) == "18446744073709551615")
      require(decoded.last_extended_start_index == 0)
      require(decoded.authority == if field == "account" do
          Some(authority)
        else
          None
        end)
    end
  end
  Ok(nil)
end

fn main() do
  case proof() do
    Ok(_) -> println("solana-lookup:ok")
    Err(error) -> panic(error)
  end
end
