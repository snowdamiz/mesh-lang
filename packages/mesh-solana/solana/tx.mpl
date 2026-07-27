from Solana.Read import Pubkey, pubkey, pubkey_string

pub struct AccountMeta do
  pubkey :: Pubkey
  signer :: Bool
  writable :: Bool
end

pub struct Instruction do
  program_id :: Pubkey
  accounts :: List < AccountMeta >
  data :: Bytes
end

struct InstructionReport do
  account_keys :: List < String >
  signer_keys :: List < String >
  writable_keys :: List < String >
end

fn string_field(value :: Json, field :: String) -> String ! String do
  case value
    |> Json.object_get(field) do
    Err( _) -> Err("SOLANA_TX: missing field #{field}")
    Ok(member) -> case member
      |> Json.as_string() do
      Err( _) -> Err("SOLANA_TX: field #{field} must be a string")
      Ok(text) -> Ok(text)
    end
  end
end

fn bool_field(value :: Json, field :: String) -> Bool ! String do
  case value
    |> Json.object_get(field) do
    Err( _) -> Err("SOLANA_TX: missing field #{field}")
    Ok(member) -> case member
      |> Json.as_bool() do
      Err( _) -> Err("SOLANA_TX: field #{field} must be a boolean")
      Ok(flag) -> Ok(flag)
    end
  end
end

fn account_meta(value :: Json) -> AccountMeta ! String do
  Ok(AccountMeta {
    pubkey : ((value
      |> string_field("pubkey")) ?
      |> pubkey()) ?,
    signer : (value
      |> bool_field("isSigner")) ?,
    writable : (value
      |> bool_field("isWritable")) ?
  })
end

fn account_metas(values :: Json, index :: Int, total :: Int, accounts :: List < AccountMeta >) -> List < AccountMeta > ! String do
  if index >= total do
    Ok(accounts)
  else
    account_metas(
      values,
      index + 1,
      total,
      accounts
        |> List.append(((values
          |> Json.array_get(index)) ?
          |> account_meta()) ?)
    )
  end
end

fn instruction_data(encoded :: String) -> Bytes ! String do
  case encoded
    |> Bytes.from_base64() do
    Err( _) -> Err("SOLANA_TX: invalid base64 data")
    Ok(data) -> if (data
      |> Bytes.to_base64()) != encoded do
      Err("SOLANA_TX: non-canonical base64 data")
    else
      if Bytes.length(data) > 1232 do
        Err("SOLANA_TX: instruction data exceeds 1232 bytes")
      else
        Ok(data)
      end
    end
  end
end

pub fn instruction_from_jupiter_json(raw :: String) -> Instruction ! String do
  let root = (raw
    |> Json.parse()) ?
  let accounts = (root
    |> Json.object_get("accounts")) ?
  let total = (accounts
    |> Json.array_length()) ?
  if total > 64 do
    Err("SOLANA_TX: instruction exceeds 64 accounts")
  else
    Ok(Instruction {
      program_id : ((root
        |> string_field("programId")) ?
        |> pubkey()) ?,
      accounts : (accounts
        |> account_metas(0, total, List.new())) ?,
      data : ((root
        |> string_field("data")) ?
        |> instruction_data()) ?
    })
  end
end

fn append_if(values :: List < String >, include :: Bool, value :: String) -> List < String > do
  if include do
    values
      |> List.append(value)
  else
    values
  end
end

fn report_accounts(accounts :: List < AccountMeta >, index :: Int, report :: InstructionReport) -> InstructionReport do
  if index >= List.length(accounts) do
    report
  else
    let account = accounts
      |> List.get(index)
    let key = account.pubkey
      |> pubkey_string()
    report_accounts(accounts, index + 1, %{report |
      account_keys : report.account_keys
        |> List.append(key),
      signer_keys : report.signer_keys
        |> append_if(account.signer, key),
      writable_keys : report.writable_keys
        |> append_if(account.writable, key)
    })
  end
end

pub fn instruction_report_json(instruction :: Instruction) -> String do
  let report = instruction.accounts
    |> report_accounts(0, InstructionReport {
      account_keys : List.new(),
      signer_keys : List.new(),
      writable_keys : List.new()
    })
  json {
    schemaVersion : 1,
    programId : instruction.program_id
      |> pubkey_string(),
    accountCount : instruction.accounts
      |> List.length(),
    accountKeys : report.account_keys,
    signerKeys : report.signer_keys,
    writableKeys : report.writable_keys,
    dataBase64 : instruction.data
      |> Bytes.to_base64(),
    dataBytes : instruction.data
      |> Bytes.length()
  }
end
