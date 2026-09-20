from Solana.Read import Hash, Pubkey, RpcRequest, RpcResponse, pubkey, pubkey_equal, pubkey_string, rpc_request, spl_token_program

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

pub struct AddressTableLookup do
  account_key :: Pubkey
  writable_indexes :: List < Int >
  readonly_indexes :: List < Int >
  writable_addresses :: List < Pubkey >
  readonly_addresses :: List < Pubkey >
end

pub struct MessageV0 do
  header :: MessageHeader
  static_account_keys :: List < Pubkey >
  recent_blockhash :: Hash
  instructions :: List < CompiledInstruction >
  address_table_lookups :: List < AddressTableLookup >
end

pub struct SimulationResult do
  slot :: U64
  succeeded :: Bool
  error_json :: String
  logs_json :: String
  units_consumed :: Option < U64 >
  replacement_blockhash_json :: String
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

struct KeyMeta do
  pubkey :: Pubkey
  signer :: Bool
  writable :: Bool
  invoked :: Bool
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

fn compute_budget_program() -> Pubkey ! String do
  "ComputeBudget111111111111111111111111111111"
    |> pubkey()
end

pub fn compute_unit_limit_instruction(units :: Int) -> Instruction ! String do
  if units < 0 || units > 4_294_967_295 do
    Err("SOLANA_TX: compute unit limit is out of u32 range")
  else
    Ok(Instruction {
      program_id : compute_budget_program() ?,
      accounts : [],
      data : (((units
        |> Int.to_string()
        |> Bytes.write_uint_le(4)) ?)
        |2> append_bytes(uint8(2) ?)) ?
    })
  end
end

pub fn compute_unit_price_instruction(micro_lamports :: U64) -> Instruction ! String do
  Ok(Instruction {
    program_id : compute_budget_program() ?,
    accounts : [],
    data : (((micro_lamports
      |> U64.to_string()
      |> Bytes.write_uint_le(8)) ?)
      |2> append_bytes(uint8(3) ?)) ?
  })
end

fn checked_account_meta(
  key :: Pubkey,
  signer :: Bool,
  writable :: Bool
) -> AccountMeta ! String do
  if Bytes.length(key.bytes) != 32 do
    Err("SOLANA_TX: instruction account key must be 32 bytes")
  else
    Ok(AccountMeta {
      pubkey : key,
      signer : signer,
      writable : writable
    })
  end
end

pub fn transfer_checked_instruction(
  source :: Pubkey,
  mint :: Pubkey,
  destination :: Pubkey,
  authority :: Pubkey,
  amount :: U64,
  decimals :: Int
) -> Instruction ! String do
  if decimals < 0 || decimals > 255 do
    Err("SOLANA_TX: token decimals are out of u8 range")
  else
    let amount_bytes = (amount
      |> U64.to_string()
      |> Bytes.write_uint_le(8)) ?
    let with_discriminator = (amount_bytes
      |2> append_bytes(uint8(12) ?)) ?
    Ok(Instruction {
      program_id : spl_token_program() ?,
      accounts : [
        checked_account_meta(source, false, true) ?,
        checked_account_meta(mint, false, false) ?,
        checked_account_meta(destination, false, true) ?,
        checked_account_meta(authority, true, false) ?
      ],
      data : append_uint8(with_discriminator, decimals) ?
    })
  end
end

pub fn create_associated_token_idempotent_instruction(
  payer :: Pubkey,
  associated_account :: Pubkey,
  wallet :: Pubkey,
  mint :: Pubkey
) -> Instruction ! String do
  let token_program = spl_token_program() ?
  Ok(Instruction {
    program_id : ("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"
      |> pubkey()) ?,
    accounts : [
      checked_account_meta(payer, true, true) ?,
      checked_account_meta(associated_account, false, true) ?,
      checked_account_meta(wallet, false, false) ?,
      checked_account_meta(mint, false, false) ?,
      checked_account_meta(("11111111111111111111111111111111"
        |> pubkey()) ?, false, false) ?,
      checked_account_meta(token_program, false, false) ?
    ],
    data : uint8(1) ?
  })
end

fn merge_key_meta(
  values :: List < KeyMeta >,
  key :: Pubkey,
  signer :: Bool,
  writable :: Bool,
  invoked :: Bool,
  index :: Int,
  found :: Bool,
  output :: List < KeyMeta >
) -> List < KeyMeta > do
  if index >= List.length(values) do
    if found do
      output
    else
      output |> List.append(KeyMeta {
        pubkey : key,
        signer : signer,
        writable : writable,
        invoked : invoked
      })
    end
  else
    let current = values
      |> List.get(index)
    if pubkey_equal(current.pubkey, key) do
      merge_key_meta(
        values,
        key,
        signer,
        writable,
        invoked,
        index + 1,
        true,
        output |> List.append(KeyMeta {
          pubkey : current.pubkey,
          signer : current.signer || signer,
          writable : current.writable || writable,
          invoked : current.invoked || invoked
        })
      )
    else
      merge_key_meta(
        values,
        key,
        signer,
        writable,
        invoked,
        index + 1,
        found,
        current
          |2> List.append(output)
      )
    end
  end
end

fn collect_account_metas(
  values :: List < KeyMeta >,
  accounts :: List < AccountMeta >,
  index :: Int
) -> List < KeyMeta > do
  if index >= List.length(accounts) do
    values
  else
    let account = accounts
      |> List.get(index)
    values
      |> merge_key_meta(
        account.pubkey,
        account.signer,
        account.writable,
        false,
        0,
        false,
        List.new()
      )
      |> collect_account_metas(accounts, index + 1)
  end
end

fn collect_instruction_metas(
  values :: List < KeyMeta >,
  instructions :: List < Instruction >,
  index :: Int
) -> List < KeyMeta > do
  if index >= List.length(instructions) do
    values
  else
    let instruction = instructions
      |> List.get(index)
    values
      |> collect_account_metas(instruction.accounts, 0)
      |> merge_key_meta(
        instruction.program_id,
        false,
        false,
        true,
        0,
        false,
        List.new()
      )
      |> collect_instruction_metas(instructions, index + 1)
  end
end

fn key_category(meta :: KeyMeta) -> Int do
  if meta.signer do
    if meta.writable do 0 else 1 end
  else
    if meta.writable do 2 else 3 end
  end
end

fn category_keys(
  values :: List < KeyMeta >,
  category :: Int,
  index :: Int,
  output :: List < Pubkey >
) -> List < Pubkey > do
  if index >= List.length(values) do
    output
  else
    let meta = values
      |> List.get(index)
    let next = if key_category(meta) == category do
      output
        |> List.append(meta.pubkey)
    else
      output
    end
    category_keys(values, category, index + 1, next)
  end
end

fn append_pubkey_values(
  output :: List < Pubkey >,
  values :: List < Pubkey >,
  index :: Int
) -> List < Pubkey > do
  if index >= List.length(values) do
    output
  else
    values
      |> List.get(index)
      |2> List.append(output)
      |> append_pubkey_values(values, index + 1)
  end
end

fn ordered_keys(values :: List < KeyMeta >) -> List < Pubkey > do
  values
    |> category_keys(0, 0, List.new())
    |> append_pubkey_values(
      category_keys(values, 1, 0, List.new()),
      0
    )
    |> append_pubkey_values(
      category_keys(values, 2, 0, List.new()),
      0
    )
    |> append_pubkey_values(
      category_keys(values, 3, 0, List.new()),
      0
    )
end

fn category_count(
  values :: List < KeyMeta >,
  category :: Int,
  index :: Int,
  count :: Int
) -> Int do
  if index >= List.length(values) do
    count
  else
    category_count(
      values,
      category,
      index + 1,
      count + if key_category(List.get(values, index)) == category do
        1
      else
        0
      end
    )
  end
end

fn header_from_metas(values :: List < KeyMeta >) -> MessageHeader do
  MessageHeader {
    num_required_signatures : category_count(values, 0, 0, 0) +
      category_count(values, 1, 0, 0),
    num_readonly_signed_accounts : category_count(values, 1, 0, 0),
    num_readonly_unsigned_accounts : category_count(values, 3, 0, 0)
  }
end

fn pubkey_index(
  values :: List < Pubkey >,
  key :: Pubkey,
  index :: Int
) -> Int ! String do
  if index >= List.length(values) do
    Err("SOLANA_TX: instruction references an uncompiled account")
  else
    if pubkey_equal(List.get(values, index), key) do
      Ok(index)
    else
      pubkey_index(values, key, index + 1)
    end
  end
end

fn compile_account_indexes(
  accounts :: List < AccountMeta >,
  keys :: List < Pubkey >,
  index :: Int,
  output :: List < Int >
) -> List < Int > ! String do
  if index >= List.length(accounts) do
    Ok(output)
  else
    compile_account_indexes(
      accounts,
      keys,
      index + 1,
      output
        |> List.append(pubkey_index(
          keys,
          List.get(accounts, index).pubkey,
          0
        ) ?)
    )
  end
end

fn compile_instructions(
  instructions :: List < Instruction >,
  keys :: List < Pubkey >,
  index :: Int,
  output :: List < CompiledInstruction >
) -> List < CompiledInstruction > ! String do
  if index >= List.length(instructions) do
    Ok(output)
  else
    let instruction = instructions
      |> List.get(index)
    compile_instructions(
      instructions,
      keys,
      index + 1,
      output
        |> List.append(CompiledInstruction {
          program_id_index : pubkey_index(
            keys,
            instruction.program_id,
            0
          ) ?,
          account_indexes : compile_account_indexes(
            instruction.accounts,
            keys,
            0,
            List.new()
          ) ?,
          data : instruction.data
        })
    )
  end
end

fn compiled_key_metas(
  payer :: Pubkey,
  instructions :: List < Instruction >
) -> List < KeyMeta > do
  [KeyMeta {
    pubkey : payer,
    signer : true,
    writable : true,
    invoked : false
  }]
    |> collect_instruction_metas(instructions, 0)
end

pub fn compile_legacy_message(
  payer :: Pubkey,
  recent_blockhash :: Hash,
  instructions :: List < Instruction >
) -> LegacyMessage ! String do
  if List.length(instructions) == 0 do
    Err("SOLANA_TX: message requires at least one instruction")
  else
    let metas = payer
      |> compiled_key_metas(instructions)
    let keys = metas
      |> ordered_keys()
    Ok(LegacyMessage {
      header : metas
        |> header_from_metas(),
      account_keys : keys,
      recent_blockhash : recent_blockhash,
      instructions : compile_instructions(
        instructions,
        keys,
        0,
        List.new()
      ) ?
    })
  end
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

fn validate_pubkeys(
  keys :: List < Pubkey >,
  index :: Int,
  label :: String
) -> Int ! String do
  if index >= List.length(keys) do
    Ok(index)
  else
    let key = keys
      |> List.get(index)
    if Bytes.length(key.bytes) != 32 do
      Err("SOLANA_TX: #{label} key must be 32 bytes")
    else
      validate_pubkeys(keys, index + 1, label)
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
  program_count :: Int,
  account_count :: Int
) -> Bytes ! String do
  if index >= List.length(instructions) do
    Ok(output)
  else
    let instruction = instructions
      |> List.get(index)
    if instruction.program_id_index < 0 || instruction.program_id_index >= program_count do
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
          program_count,
          account_count
        )
      end
    end
  end
end

fn validate_header(header :: MessageHeader, account_count :: Int) -> Int ! String do
  if account_count > 256 do
    Err("SOLANA_TX: message exceeds 256 static account keys")
  else
    if header.num_required_signatures < 0 || header.num_required_signatures > account_count || header.num_readonly_signed_accounts < 0 || header.num_readonly_signed_accounts > header.num_required_signatures || header.num_readonly_unsigned_accounts < 0 || header.num_readonly_unsigned_accounts > account_count - header.num_required_signatures do
      Err("SOLANA_TX: invalid message header")
    else
      Ok(account_count)
    end
  end
end

fn legacy_account_count(message :: LegacyMessage) -> Int ! String do
  let account_count = message.account_keys
    |> List.length()
  validate_header(message.header, account_count) ?
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

pub fn serialize_legacy_message(message :: LegacyMessage) -> Bytes ! String do
  let account_count = (message
    |> legacy_account_count()) ?
  let with_signatures = append_uint8(
    Bytes.empty(),
    message.header.num_required_signatures
  ) ?
  let with_readonly_signers = append_uint8(
    with_signatures,
    message.header.num_readonly_signed_accounts
  ) ?
  let with_readonly_unsigned = append_uint8(
    with_readonly_signers,
    message.header.num_readonly_unsigned_accounts
  ) ?
  let with_account_count = append_short_u16(
    with_readonly_unsigned,
    account_count
  ) ?
  let with_account_keys = append_pubkeys(
    with_account_count,
    message.account_keys,
    0
  ) ?
  let with_blockhash = append_bytes(
    with_account_keys,
    message.recent_blockhash.bytes
  ) ?
  let with_instruction_count = append_short_u16(
    with_blockhash,
    List.length(message.instructions)
  ) ?
  let bytes = append_compiled_instructions(
    with_instruction_count,
    message.instructions,
    0,
    account_count,
    account_count
  ) ?
  if Bytes.length(bytes) > 1232 do
    Err("SOLANA_TX: serialized message exceeds 1232 bytes")
  else
    Ok(bytes)
  end
end

fn loaded_account_count(
  lookups :: List < AddressTableLookup >,
  index :: Int,
  total :: Int
) -> Int ! String do
  if index >= List.length(lookups) do
    Ok(total)
  else
    let lookup = lookups
      |> List.get(index)
    let writable_count = lookup.writable_indexes
      |> List.length()
    let readonly_count = lookup.readonly_indexes
      |> List.length()
    if Bytes.length(lookup.account_key.bytes) != 32 do
      Err("SOLANA_TX: address lookup table key must be 32 bytes")
    else if writable_count != List.length(lookup.writable_addresses) || readonly_count != List.length(lookup.readonly_addresses) do
      Err("SOLANA_TX: address lookup indexes and resolved addresses differ")
    else
      (lookup.writable_addresses
        |> validate_pubkeys(0, "loaded writable account")) ?
      (lookup.readonly_addresses
        |> validate_pubkeys(0, "loaded readonly account")) ?
      if writable_count > 256 || readonly_count > 256 do
        Err("SOLANA_TX: address lookup exceeds 256 indexes")
      else
        loaded_account_count(
          lookups,
          index + 1,
          total + writable_count + readonly_count
        )
      end
    end
  end
end

fn pubkey_occurrences(
  values :: List < Pubkey >,
  key :: Pubkey,
  index :: Int,
  count :: Int
) -> Int do
  if index >= List.length(values) do
    count
  else
    pubkey_occurrences(
      values,
      key,
      index + 1,
      count + if pubkey_equal(List.get(values, index), key) do 1 else 0 end
    )
  end
end

fn lookup_occurrences(
  lookups :: List < AddressTableLookup >,
  key :: Pubkey,
  index :: Int,
  count :: Int
) -> Int do
  if index >= List.length(lookups) do
    count
  else
    let lookup = lookups
      |> List.get(index)
    lookup_occurrences(
      lookups,
      key,
      index + 1,
      count +
        pubkey_occurrences(lookup.writable_addresses, key, 0, 0) +
        pubkey_occurrences(lookup.readonly_addresses, key, 0, 0)
    )
  end
end

fn key_meta_index(
  values :: List < KeyMeta >,
  key :: Pubkey,
  index :: Int
) -> Int do
  if index >= List.length(values) do
    -1
  else
    if pubkey_equal(List.get(values, index).pubkey, key) do
      index
    else
      key_meta_index(values, key, index + 1)
    end
  end
end

fn validate_loaded_addresses(
  metas :: List < KeyMeta >,
  lookups :: List < AddressTableLookup >,
  addresses :: List < Pubkey >,
  writable :: Bool,
  index :: Int
) -> Int ! String do
  if index >= List.length(addresses) do
    Ok(index)
  else
    let key = addresses
      |> List.get(index)
    if lookup_occurrences(lookups, key, 0, 0) != 1 do
      Err("SOLANA_TX: loaded account appears more than once")
    else
      let meta_index = key
        |2> key_meta_index(metas, 0)
      if meta_index < 0 do
        validate_loaded_addresses(
          metas,
          lookups,
          addresses,
          writable,
          index + 1
        )
      else
        let meta = metas
          |> List.get(meta_index)
        if meta.signer || meta.invoked do
          Err("SOLANA_TX: signer or program account cannot be loaded")
        else if meta.writable != writable do
          Err("SOLANA_TX: loaded account privilege differs from instruction")
        else
          validate_loaded_addresses(
            metas,
            lookups,
            addresses,
            writable,
            index + 1
          )
        end
      end
    end
  end
end

fn validate_loaded_metas(
  metas :: List < KeyMeta >,
  lookups :: List < AddressTableLookup >,
  index :: Int
) -> Int ! String do
  if index >= List.length(lookups) do
    Ok(index)
  else
    let lookup = lookups
      |> List.get(index)
    validate_loaded_addresses(
      metas,
      lookups,
      lookup.writable_addresses,
      true,
      0
    ) ?
    validate_loaded_addresses(
      metas,
      lookups,
      lookup.readonly_addresses,
      false,
      0
    ) ?
    validate_loaded_metas(metas, lookups, index + 1)
  end
end

fn static_key_metas(
  metas :: List < KeyMeta >,
  lookups :: List < AddressTableLookup >,
  index :: Int,
  output :: List < KeyMeta >
) -> List < KeyMeta > do
  if index >= List.length(metas) do
    output
  else
    let meta = metas
      |> List.get(index)
    static_key_metas(
      metas,
      lookups,
      index + 1,
      if lookup_occurrences(lookups, meta.pubkey, 0, 0) == 0 do
        output
          |> List.append(meta)
      else
        output
      end
    )
  end
end

fn loaded_pubkeys(
  lookups :: List < AddressTableLookup >,
  writable :: Bool,
  index :: Int,
  output :: List < Pubkey >
) -> List < Pubkey > do
  if index >= List.length(lookups) do
    output
  else
    let lookup = lookups
      |> List.get(index)
    output
      |> append_pubkey_values(
        if writable do
          lookup.writable_addresses
        else
          lookup.readonly_addresses
        end,
        0
      )
      |4> loaded_pubkeys(lookups, writable, index + 1)
  end
end

pub fn compile_message_v0(
  payer :: Pubkey,
  recent_blockhash :: Hash,
  instructions :: List < Instruction >,
  address_table_lookups :: List < AddressTableLookup >
) -> MessageV0 ! String do
  if List.length(instructions) == 0 do
    Err("SOLANA_TX: message requires at least one instruction")
  else
    let metas = payer
      |> compiled_key_metas(instructions)
    let loaded_count = loaded_account_count(
      address_table_lookups,
      0,
      0
    ) ?
    validate_loaded_metas(metas, address_table_lookups, 0) ?
    let static_metas = static_key_metas(
      metas,
      address_table_lookups,
      0,
      List.new()
    )
    let static_keys = static_metas
      |> ordered_keys()
    let keys = static_keys
      |> append_pubkey_values(
        loaded_pubkeys(
          address_table_lookups,
          true,
          0,
          List.new()
        ),
        0
      )
      |> append_pubkey_values(
        loaded_pubkeys(
          address_table_lookups,
          false,
          0,
          List.new()
        ),
        0
      )
    if List.length(static_keys) + loaded_count > 256 do
      Err("SOLANA_TX: v0 message exceeds 256 resolved accounts")
    else
      Ok(MessageV0 {
        header : static_metas
          |> header_from_metas(),
        static_account_keys : static_keys,
        recent_blockhash : recent_blockhash,
        instructions : compile_instructions(
          instructions,
          keys,
          0,
          List.new()
        ) ?,
        address_table_lookups : address_table_lookups
      })
    end
  end
end

fn append_lookup_indexes(
  output :: Bytes,
  indexes :: List < Int >,
  index :: Int
) -> Bytes ! String do
  if index >= List.length(indexes) do
    Ok(output)
  else
    let lookup_index = indexes
      |> List.get(index)
    if lookup_index < 0 || lookup_index > 255 do
      Err("SOLANA_TX: address lookup index is out of range")
    else
      append_lookup_indexes(
        append_uint8(output, lookup_index) ?,
        indexes,
        index + 1
      )
    end
  end
end

fn append_address_table_lookups(
  output :: Bytes,
  lookups :: List < AddressTableLookup >,
  index :: Int
) -> Bytes ! String do
  if index >= List.length(lookups) do
    Ok(output)
  else
    let lookup = lookups
      |> List.get(index)
    let with_key = append_bytes(output, lookup.account_key.bytes) ?
    let with_writable_count = append_short_u16(
      with_key,
      List.length(lookup.writable_indexes)
    ) ?
    let with_writable_indexes = append_lookup_indexes(
      with_writable_count,
      lookup.writable_indexes,
      0
    ) ?
    let with_readonly_count = append_short_u16(
      with_writable_indexes,
      List.length(lookup.readonly_indexes)
    ) ?
    append_address_table_lookups(
      append_lookup_indexes(
        with_readonly_count,
        lookup.readonly_indexes,
        0
      ) ?,
      lookups,
      index + 1
    )
  end
end

fn message_v0_account_count(message :: MessageV0) -> Int ! String do
  let static_count = message.static_account_keys
    |> List.length()
  validate_header(message.header, static_count) ?
  if List.length(message.instructions) > 256 || List.length(message.address_table_lookups) > 256 do
    Err("SOLANA_TX: v0 message exceeds 256 instructions or lookups")
  else
    if Bytes.length(message.recent_blockhash.bytes) != 32 do
      Err("SOLANA_TX: recent blockhash must be 32 bytes")
    else
      let account_count = static_count + (loaded_account_count(
        message.address_table_lookups,
        0,
        0
      ) ?)
      if account_count > 256 do
        Err("SOLANA_TX: v0 message exceeds 256 resolved accounts")
      else
        Ok(account_count)
      end
    end
  end
end

pub fn serialize_message_v0(message :: MessageV0) -> Bytes ! String do
  let static_count = message.static_account_keys
    |> List.length()
  let account_count = (message
    |> message_v0_account_count()) ?
  let versioned = append_uint8(Bytes.empty(), 128) ?
  let with_signatures = append_uint8(
    versioned,
    message.header.num_required_signatures
  ) ?
  let with_readonly_signers = append_uint8(
    with_signatures,
    message.header.num_readonly_signed_accounts
  ) ?
  let with_readonly_unsigned = append_uint8(
    with_readonly_signers,
    message.header.num_readonly_unsigned_accounts
  ) ?
  let with_static_count = append_short_u16(
    with_readonly_unsigned,
    static_count
  ) ?
  let with_static_keys = append_pubkeys(
    with_static_count,
    message.static_account_keys,
    0
  ) ?
  let with_blockhash = append_bytes(
    with_static_keys,
    message.recent_blockhash.bytes
  ) ?
  let with_instruction_count = append_short_u16(
    with_blockhash,
    List.length(message.instructions)
  ) ?
  let with_instructions = append_compiled_instructions(
    with_instruction_count,
    message.instructions,
    0,
    static_count,
    account_count
  ) ?
  let with_lookup_count = append_short_u16(
    with_instructions,
    List.length(message.address_table_lookups)
  ) ?
  let bytes = append_address_table_lookups(
    with_lookup_count,
    message.address_table_lookups,
    0
  ) ?
  if Bytes.length(bytes) > 1232 do
    Err("SOLANA_TX: serialized message exceeds 1232 bytes")
  else
    Ok(bytes)
  end
end

fn append_zero_bytes(output :: Bytes, remaining :: Int) -> Bytes ! String do
  if remaining <= 0 do
    Ok(output)
  else
    append_zero_bytes(
      append_uint8(output, 0) ?,
      remaining - 1
    )
  end
end

fn serialize_unsigned_transaction(
  message :: Bytes,
  count :: Int
) -> Bytes ! String do
  if count < 0 || count > 256 do
    Err("SOLANA_TX: required signature count is out of range")
  else
    if Bytes.length(message) == 0 do
      Err("SOLANA_TX: transaction message must not be empty")
    else
      let with_count = append_short_u16(
        Bytes.empty(),
        count
      ) ?
      let with_signatures = append_zero_bytes(
        with_count,
        count * 64
      ) ?
      let bytes = append_bytes(with_signatures, message) ?
      if Bytes.length(bytes) > 1232 do
        Err("SOLANA_TX: serialized transaction exceeds 1232 bytes")
      else
        Ok(bytes)
      end
    end
  end
end

pub fn serialize_unsigned_legacy_transaction(
  message :: LegacyMessage
) -> Bytes ! String do
  serialize_unsigned_transaction(
    serialize_legacy_message(message) ?,
    message.header.num_required_signatures
  )
end

pub fn serialize_unsigned_v0_transaction(
  message :: MessageV0
) -> Bytes ! String do
  serialize_unsigned_transaction(
    serialize_message_v0(message) ?,
    message.header.num_required_signatures
  )
end

fn simulation_commitment(value :: String) -> String ! String do
  if value == "processed" || value == "confirmed" || value == "finalized" do
    Ok(value)
  else
    Err("SOLANA_TX: unsupported simulation commitment #{value}")
  end
end

fn min_context_slot_json(value :: Option < U64 >) -> String do
  case value do
    None -> ""
    Some(slot) -> ",\"minContextSlot\":#{U64.to_string(slot)}"
  end
end

pub fn simulate_transaction_request(
  id :: Int,
  transaction :: Bytes,
  commitment :: String,
  replace_recent_blockhash :: Bool,
  min_context_slot :: Option < U64 >
) -> RpcRequest ! String do
  if Bytes.length(transaction) == 0 || Bytes.length(transaction) > 1232 do
    Err("SOLANA_TX: simulation transaction size is out of range")
  else
    let replacement = if replace_recent_blockhash do
      "true"
    else
      "false"
    end
    rpc_request(
      id,
      "simulateTransaction",
      "[#{Json.encode_string(Bytes.to_base64(transaction))},{\"commitment\":#{Json.encode_string(simulation_commitment(commitment) ?)},\"encoding\":\"base64\",\"sigVerify\":false,\"replaceRecentBlockhash\":#{replacement}#{min_context_slot_json(min_context_slot)}}]"
    )
  end
end

pub fn simulation_result(response :: RpcResponse) -> SimulationResult ! String do
  if !response.ok do
    Err("SOLANA_TX: simulation RPC error #{response.error_json}")
  else
    let context_json = Json.get(response.result_json, "context")
    let value_json = Json.get(response.result_json, "value")
    if context_json == "" || value_json == "" do
      Err("SOLANA_TX: simulation result is missing context or value")
    else
      let slot_text = Json.get(context_json, "slot")
      let parsed_value = (value_json
        |> Json.parse()) ?
      let error_value = (parsed_value
        |> Json.object_get("err")) ?
      let logs = Json.get(value_json, "logs")
      let units = Json.get(value_json, "unitsConsumed")
      let replacement = Json.get(value_json, "replacementBlockhash")
      if slot_text == "" || logs == "" do
        Err("SOLANA_TX: simulation result is missing slot or logs")
      else
        let units_consumed = if units == "" || units == "null" do
          None
        else
          Some((units
            |> U64.parse()) ?)
        end
        Ok(SimulationResult {
          slot : (slot_text
            |> U64.parse()) ?,
          succeeded : error_value
            |> Json.is_null(),
          error_json : if error_value
            |> Json.is_null() do
            ""
          else
            error_value
              |> Json.encode()
          end,
          logs_json : logs,
          units_consumed : units_consumed,
          replacement_blockhash_json : if replacement == "" do
            "null"
          else
            replacement
          end
        })
      end
    end
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

fn append_instruction_values(
  output :: List < Instruction >,
  values :: List < Instruction >,
  index :: Int
) -> List < Instruction > do
  if index >= List.length(values) do
    output
  else
    values
      |> List.get(index)
      |2> List.append(output)
      |> append_instruction_values(values, index + 1)
  end
end

fn append_optional_instruction(
  output :: List < Instruction >,
  value :: Option < Instruction >
) -> List < Instruction > do
  case value do
    None -> output
    Some(instruction) -> output
      |> List.append(instruction)
  end
end

pub fn jupiter_instructions(
  value :: JupiterInstructionSet
) -> List < Instruction > do
  List.new()
    |> append_instruction_values(value.compute_budget, 0)
    |> append_instruction_values(value.setup, 0)
    |> List.append(value.swap)
    |> append_optional_instruction(value.cleanup)
    |> append_instruction_values(value.other, 0)
    |> append_optional_instruction(value.tip)
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

fn pubkey_strings(
  keys :: List < Pubkey >,
  index :: Int,
  values :: List < String >
) -> List < String > do
  if index >= List.length(keys) do
    values
  else
    keys
      |> List.get(index)
      |> pubkey_string()
      |2> List.append(values)
      |3> pubkey_strings(keys, index + 1)
  end
end

fn compiled_program_ids(
  keys :: List < Pubkey >,
  instructions :: List < CompiledInstruction >,
  index :: Int,
  values :: List < String >
) -> List < String > do
  if index >= List.length(instructions) do
    values
  else
    let instruction = instructions
      |> List.get(index)
    keys
      |> List.get(instruction.program_id_index)
      |> pubkey_string()
      |2> append_unique(values)
      |4> compiled_program_ids(keys, instructions, index + 1)
  end
end

fn lookup_table_keys(
  lookups :: List < AddressTableLookup >,
  index :: Int,
  keys :: List < String >
) -> List < String > do
  if index >= List.length(lookups) do
    keys
  else
    let lookup = lookups
      |> List.get(index)
    lookup.account_key
      |> pubkey_string()
      |2> List.append(keys)
      |3> lookup_table_keys(lookups, index + 1)
  end
end

fn loaded_address_strings(
  lookups :: List < AddressTableLookup >,
  index :: Int,
  writable :: Bool,
  keys :: List < String >
) -> List < String > do
  if index >= List.length(lookups) do
    keys
  else
    let lookup = lookups
      |> List.get(index)
    let addresses = if writable do
      lookup.writable_addresses
    else
      lookup.readonly_addresses
    end
    addresses
      |> pubkey_strings(0, keys)
      |4> loaded_address_strings(lookups, index + 1, writable)
  end
end

fn loaded_accounts(
  lookups :: List < AddressTableLookup >,
  index :: Int,
  writable :: Int,
  readonly :: Int
) -> List < Int > do
  if index >= List.length(lookups) do
    [writable, readonly]
  else
    let lookup = lookups
      |> List.get(index)
    loaded_accounts(
      lookups,
      index + 1,
      writable + List.length(lookup.writable_indexes),
      readonly + List.length(lookup.readonly_indexes)
    )
  end
end

pub fn legacy_message_report_json(
  message :: LegacyMessage
) -> String ! String do
  let bytes = (message
    |> serialize_legacy_message()) ?
  Ok(json {
    schemaVersion : 1,
    version : "legacy",
    requiredSignatures : message.header.num_required_signatures,
    accountKeys : pubkey_strings(message.account_keys, 0, List.new()),
    programIds : compiled_program_ids(
      message.account_keys,
      message.instructions,
      0,
      List.new()
    ),
    instructionCount : List.length(message.instructions),
    lookupTableKeys : List.new(),
    loadedWritableAccounts : 0,
    loadedReadonlyAccounts : 0,
    messageBytes : Bytes.length(bytes)
  })
end

pub fn message_v0_report_json(
  message :: MessageV0
) -> String ! String do
  let bytes = (message
    |> serialize_message_v0()) ?
  let loaded = loaded_accounts(
    message.address_table_lookups,
    0,
    0,
    0
  )
  let static_keys = pubkey_strings(
    message.static_account_keys,
    0,
    List.new()
  )
  let writable_keys = loaded_address_strings(
    message.address_table_lookups,
    0,
    true,
    List.new()
  )
  let readonly_keys = loaded_address_strings(
    message.address_table_lookups,
    0,
    false,
    List.new()
  )
  Ok(json {
    schemaVersion : 1,
    version : "v0",
    requiredSignatures : message.header.num_required_signatures,
    staticAccountKeys : static_keys,
    accountKeys : loaded_address_strings(
      message.address_table_lookups,
      0,
      false,
      loaded_address_strings(
        message.address_table_lookups,
        0,
        true,
        static_keys
      )
    ),
    programIds : compiled_program_ids(
      message.static_account_keys,
      message.instructions,
      0,
      List.new()
    ),
    instructionCount : List.length(message.instructions),
    lookupTableKeys : lookup_table_keys(
      message.address_table_lookups,
      0,
      List.new()
    ),
    loadedWritableAccounts : List.get(loaded, 0),
    loadedReadonlyAccounts : List.get(loaded, 1),
    loadedWritableAccountKeys : writable_keys,
    loadedReadonlyAccountKeys : readonly_keys,
    messageBytes : Bytes.length(bytes)
  })
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
