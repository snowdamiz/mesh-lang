from Solana.Read import Hash, Pubkey, pubkey, pubkey_string

pub struct MessageHeader do
  num_required_signatures :: Int
  num_readonly_signed_accounts :: Int
  num_readonly_unsigned_accounts :: Int
end

pub struct CompiledInstruction do
  program_id_index :: Int
  account_indexes :: List < Int >
  data :: Bytes
end

pub struct LegacyMessage do
  header :: MessageHeader
  account_keys :: List < Pubkey >
  recent_blockhash :: Hash
  instructions :: List < CompiledInstruction >
end

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

fn uint8(value :: Int) -> Bytes ! String do
  value
    |> Int.to_string()
    |> Bytes.write_uint_le(1)
end

fn append_bytes(output :: Bytes, value :: Bytes) -> Bytes ! String do
  value |2> Bytes.concat(output)
end

fn append_uint8(output :: Bytes, value :: Int) -> Bytes ! String do
  append_bytes(output, uint8(value) ?)
end

fn short_u16(value :: Int) -> Bytes ! String do
  if value < 0 || value > 65_535 do
    Err("SOLANA_TX: compact-u16 value is out of range")
  else
    if value < 128 do
      uint8(value)
    else
      if value < 16_384 do
        append_uint8(uint8((value % 128) + 128) ?, value / 128)
      else
        let output = append_uint8(
          uint8((value % 128) + 128) ?,
          ((value / 128) % 128) + 128
        ) ?
        append_uint8(output, value / 16_384)
      end
    end
  end
end

fn append_short_u16(output :: Bytes, value :: Int) -> Bytes ! String do
  append_bytes(output, short_u16(value) ?)
end

fn append_pubkeys(output :: Bytes, keys :: List < Pubkey >, index :: Int) -> Bytes ! String do
  if index >= List.length(keys) do
    Ok(output)
  else
    let key = keys
      |> List.get(index)
    if Bytes.length(key.bytes) != 32 do
      Err("SOLANA_TX: account key must be 32 bytes")
    else
      append_pubkeys(
        append_bytes(output, key.bytes) ?,
        keys,
        index + 1
      )
    end
  end
end

fn append_account_indexes(
  output :: Bytes,
  indexes :: List < Int >,
  index :: Int,
  account_count :: Int
) -> Bytes ! String do
  if index >= List.length(indexes) do
    Ok(output)
  else
    let account_index = indexes
      |> List.get(index)
    if account_index < 0 || account_index >= account_count do
      Err("SOLANA_TX: instruction account index is out of range")
    else
      append_account_indexes(
        append_uint8(output, account_index) ?,
        indexes,
        index + 1,
        account_count
      )
    end
  end
end

fn append_compiled_instructions(
  output :: Bytes,
  instructions :: List < CompiledInstruction >,
  index :: Int,
  account_count :: Int
) -> Bytes ! String do
  if index >= List.length(instructions) do
    Ok(output)
  else
    let instruction = instructions
      |> List.get(index)
    if instruction.program_id_index < 0 || instruction.program_id_index >= account_count do
      Err("SOLANA_TX: instruction program index is out of range")
    else
      if List.length(instruction.account_indexes) > 256 do
        Err("SOLANA_TX: instruction exceeds 256 account indexes")
      else
        let with_program = append_uint8(
          output,
          instruction.program_id_index
        ) ?
        let with_account_count = append_short_u16(
          with_program,
          List.length(instruction.account_indexes)
        ) ?
        let with_accounts = append_account_indexes(
          with_account_count,
          instruction.account_indexes,
          0,
          account_count
        ) ?
        let with_data_count = append_short_u16(
          with_accounts,
          Bytes.length(instruction.data)
        ) ?
        append_compiled_instructions(
          append_bytes(with_data_count, instruction.data) ?,
          instructions,
          index + 1,
          account_count
        )
      end
    end
  end
end

fn legacy_account_count(message :: LegacyMessage) -> Int ! String do
  let account_count = message.account_keys
    |> List.length()
  let header = message.header
  if account_count > 256 do
    Err("SOLANA_TX: legacy message exceeds 256 account keys")
  else
    if header.num_required_signatures < 0 || header.num_required_signatures > account_count || header.num_readonly_signed_accounts < 0 || header.num_readonly_signed_accounts > header.num_required_signatures || header.num_readonly_unsigned_accounts < 0 || header.num_readonly_unsigned_accounts > account_count - header.num_required_signatures do
      Err("SOLANA_TX: invalid legacy message header")
    else
      if List.length(message.instructions) > 256 do
        Err("SOLANA_TX: legacy message exceeds 256 instructions")
      else
        if Bytes.length(message.recent_blockhash.bytes) != 32 do
          Err("SOLANA_TX: recent blockhash must be 32 bytes")
        else
          Ok(account_count)
        end
      end
    end
  end
end

pub fn serialize_legacy_message(message :: LegacyMessage) -> Bytes ! String do
  let account_count = (message
    |> legacy_account_count()) ?
  let bytes = append_uint8(
    Bytes.empty(),
    message.header.num_required_signatures
  ) ?
  let bytes = append_uint8(
    bytes,
    message.header.num_readonly_signed_accounts
  ) ?
  let bytes = append_uint8(
    bytes,
    message.header.num_readonly_unsigned_accounts
  ) ?
  let bytes = append_short_u16(bytes, account_count) ?
  let bytes = append_pubkeys(bytes, message.account_keys, 0) ?
  let bytes = append_bytes(bytes, message.recent_blockhash.bytes) ?
  let bytes = append_short_u16(
    bytes,
    List.length(message.instructions)
  ) ?
  let bytes = append_compiled_instructions(
    bytes,
    message.instructions,
    0,
    account_count
  ) ?
  if Bytes.length(bytes) > 1232 do
    Err("SOLANA_TX: serialized message exceeds 1232 bytes")
  else
    Ok(bytes)
  end
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
