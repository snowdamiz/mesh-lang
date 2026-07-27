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

pub struct JupiterInstructionSet do
  compute_budget :: List < Instruction >
  setup :: List < Instruction >
  other :: List < Instruction >
  swap :: Instruction
  cleanup :: Option < Instruction >
  tip :: Option < Instruction >
end

struct InstructionReport do
  account_keys :: List < String >
  signer_keys :: List < String >
  writable_keys :: List < String >
end

struct JupiterInstructionReport do
  instruction_count :: Int
  data_bytes :: Int
  program_ids :: List < String >
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

fn instruction_list(values :: Json, index :: Int, total :: Int, instructions :: List < Instruction >) -> List < Instruction > ! String do
  if index >= total do
    Ok(instructions)
  else
    instruction_list(
      values,
      index + 1,
      total,
      instructions
        |> List.append(((values
          |> Json.array_get(index)) ?
          |> Json.encode()
          |> instruction_from_jupiter_json()) ?)
    )
  end
end

fn instruction_array_field(root :: Json, field :: String) -> List < Instruction > ! String do
  case root
    |> Json.object_get(field) do
    Err( _) -> Err("SOLANA_TX: missing field #{field}")
    Ok(values) -> case values
      |> Json.array_length() do
      Err( _) -> Err("SOLANA_TX: field #{field} must be an array")
      Ok(total) -> if total > 64 do
        Err("SOLANA_TX: field #{field} exceeds 64 instructions")
      else
        instruction_list(values, 0, total, List.new())
      end
    end
  end
end

fn instruction_field(root :: Json, field :: String) -> Instruction ! String do
  case root
    |> Json.object_get(field) do
    Err( _) -> Err("SOLANA_TX: missing field #{field}")
    Ok(value) -> value
      |> Json.encode()
      |> instruction_from_jupiter_json()
  end
end

fn optional_instruction_field(root :: Json, field :: String) -> Option < Instruction > ! String do
  case root
    |> Json.object_get(field) do
    Err( _) -> Ok(None)
    Ok(value) -> if value
      |> Json.is_null() do
      Ok(None)
    else
      Ok(Some((value
        |> Json.encode()
        |> instruction_from_jupiter_json()) ?))
    end
  end
end

fn option_count(value :: Option < Instruction >) -> Int do
  case value do
    None -> 0
    Some( _) -> 1
  end
end

fn add_count(total :: Int, value :: Int) -> Int do
  total + value
end

fn instruction_set_count(value :: JupiterInstructionSet) -> Int do
  1
    |> add_count(List.length(value.compute_budget))
    |> add_count(List.length(value.setup))
    |> add_count(List.length(value.other))
    |> add_count(option_count(value.cleanup))
    |> add_count(option_count(value.tip))
end

pub fn jupiter_instruction_set_from_json(raw :: String) -> JupiterInstructionSet ! String do
  let root = (raw
    |> Json.parse()) ?
  let instructions = JupiterInstructionSet {
    compute_budget : (root
      |> instruction_array_field("computeBudgetInstructions")) ?,
    setup : (root
      |> instruction_array_field("setupInstructions")) ?,
    other : (root
      |> instruction_array_field("otherInstructions")) ?,
    swap : (root
      |> instruction_field("swapInstruction")) ?,
    cleanup : (root
      |> optional_instruction_field("cleanupInstruction")) ?,
    tip : (root
      |> optional_instruction_field("tipInstruction")) ?
  }
  if instruction_set_count(instructions) > 64 do
    Err("SOLANA_TX: Jupiter instruction set exceeds 64 instructions")
  else
    Ok(instructions)
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

fn append_unique(values :: List < String >, value :: String) -> List < String > do
  if List.contains(values, value) do
    values
  else
    values
      |> List.append(value)
  end
end

fn collect_account(report :: JupiterInstructionReport, account :: AccountMeta) -> JupiterInstructionReport do
  let key = account.pubkey
    |> pubkey_string()
  %{report |
    account_keys : report.account_keys
      |> append_unique(key),
    signer_keys : report.signer_keys
      |> append_unique_if(account.signer, key),
    writable_keys : report.writable_keys
      |> append_unique_if(account.writable, key)
  }
end

fn append_unique_if(values :: List < String >, include :: Bool, value :: String) -> List < String > do
  if include do
    values
      |> append_unique(value)
  else
    values
  end
end

fn collect_accounts(report :: JupiterInstructionReport, accounts :: List < AccountMeta >, index :: Int) -> JupiterInstructionReport do
  if index >= List.length(accounts) do
    report
  else
    report
      |> collect_account(List.get(accounts, index))
      |> collect_accounts(accounts, index + 1)
  end
end

fn collect_instruction(report :: JupiterInstructionReport, instruction :: Instruction) -> JupiterInstructionReport do
  %{report |
    instruction_count : report.instruction_count + 1,
    data_bytes : report.data_bytes + Bytes.length(instruction.data),
    program_ids : report.program_ids
      |> append_unique(instruction.program_id
        |> pubkey_string())
  }
    |> collect_accounts(instruction.accounts, 0)
end

fn collect_instructions(report :: JupiterInstructionReport, instructions :: List < Instruction >, index :: Int) -> JupiterInstructionReport do
  if index >= List.length(instructions) do
    report
  else
    report
      |> collect_instruction(List.get(instructions, index))
      |> collect_instructions(instructions, index + 1)
  end
end

fn collect_optional_instruction(report :: JupiterInstructionReport, instruction :: Option < Instruction >) -> JupiterInstructionReport do
  case instruction do
    None -> report
    Some(value) -> report
      |> collect_instruction(value)
  end
end

pub fn jupiter_instruction_set_report_json(instructions :: JupiterInstructionSet) -> String do
  let report = JupiterInstructionReport {
    instruction_count : 0,
    data_bytes : 0,
    program_ids : List.new(),
    account_keys : List.new(),
    signer_keys : List.new(),
    writable_keys : List.new()
  }
    |> collect_instructions(instructions.compute_budget, 0)
    |> collect_instructions(instructions.setup, 0)
    |> collect_instructions(instructions.other, 0)
    |> collect_instruction(instructions.swap)
    |> collect_optional_instruction(instructions.cleanup)
    |> collect_optional_instruction(instructions.tip)
  json {
    schemaVersion : 1,
    source : "jupiter-build",
    instructionCount : report.instruction_count,
    computeBudgetCount : List.length(instructions.compute_budget),
    setupCount : List.length(instructions.setup),
    otherCount : List.length(instructions.other),
    cleanupCount : option_count(instructions.cleanup),
    tipCount : option_count(instructions.tip),
    dataBytes : report.data_bytes,
    programIds : report.program_ids,
    accountKeys : report.account_keys,
    signerKeys : report.signer_keys,
    writableKeys : report.writable_keys
  }
end
