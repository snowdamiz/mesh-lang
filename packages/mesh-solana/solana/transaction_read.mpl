from Solana.Read import Pubkey, RpcRequest, RpcResponse, pubkey, pubkey_equal, rpc_request, rpc_send
from Solana.Tx import AccountMeta, Instruction, MessageHeader

# Addresses in this bounded reader are all in the signed message. Version 0
# lookup tables are rejected explicitly rather than trusting unsigned RPC keys.
pub struct SignedTransaction do
  signature :: Bytes
  signatures :: List<Bytes>
  version :: Int
  header :: MessageHeader
  accounts :: List<AccountMeta>
  instructions :: List<Instruction>
  message :: Bytes
  recent_blockhash :: Bytes
  wire :: Bytes
end

pub struct TransactionTokenBalance do
  account_index :: Int
  mint :: Pubkey
  owner :: Option<Pubkey>
  program_id :: Option<Pubkey>
  amount :: U64
  decimals :: Int
end

pub struct TransactionInnerGroup do
  instruction_index :: Int
  instructions :: List<Instruction>
end

pub struct FinalizedTransaction do
  signed :: SignedTransaction
  slot :: U64
  block_time :: Option<Int>
  fee :: U64
  pre_balances :: List<U64>
  post_balances :: List<U64>
  pre_token_balances :: List<TransactionTokenBalance>
  post_token_balances :: List<TransactionTokenBalance>
  # -1 means RPC did not record inner instructions. A payment can reject it.
  inner_instruction_count :: Int
  inner_instructions :: List<TransactionInnerGroup>
end

pub struct FinalizedTransactionOutcome do
  transaction :: FinalizedTransaction
  # Canonical supported TransactionError JSON; None is successful execution.
  execution_error :: Option<String>
end

struct Cursor do
  bytes :: Bytes
  offset :: Int
end

fn byte(cursor :: Cursor) -> Result<(Cursor, Int), String> do
  let value = Bytes.get(cursor.bytes, cursor.offset)?
  Ok((Cursor { bytes: cursor.bytes, offset: cursor.offset + 1 }, value))
end

fn fixed(cursor :: Cursor, length :: Int) -> Result<(Cursor, Bytes), String> do
  let value = Bytes.slice(cursor.bytes, cursor.offset, length)?
  Ok((Cursor { bytes: cursor.bytes, offset: cursor.offset + length }, value))
end

# Solana ShortU16 is canonical: no redundant final zero, no fourth byte,
# and only the low two bits may be set in its third byte.
fn compact(cursor :: Cursor) -> Result<(Cursor, Int), String> do
  let (next, first) = byte(cursor)?
  if first < 128 do
    Ok((next, first))
  else
    let (last, second) = byte(next)?
    if second < 128 do
      if second == 0 do
        return Err("SOLANA_TRANSACTION: noncanonical short vector")
      end
      Ok((last, first - 128 + second * 128))
    else
      let (next_cursor, third) = byte(last)?
      if third < 1 || third > 3 do
        return Err("SOLANA_TRANSACTION: invalid short vector")
      end
      Ok((next_cursor, first - 128 + (second - 128) * 128 + third * 16384))
    end
  end
end

struct ByteVector do
  cursor :: Cursor
  values :: List<Bytes>
end

fn fixed_vector(cursor :: Cursor,
  count :: Int,
  width :: Int,
  values :: List<Bytes>) -> ByteVector!String do
  if count == 0 do
    Ok(ByteVector { cursor: cursor, values: values })
  else
    let (next, value) = fixed(cursor, width)?
    fixed_vector(next, count - 1, width, List.append(values, value))
  end
end

struct IndexVector do
  cursor :: Cursor
  values :: List<Int>
end

fn indexes(cursor :: Cursor,
  count :: Int,
  bound :: Int,
  values :: List<Int>) -> IndexVector!String do
  if count == 0 do
    Ok(IndexVector { cursor: cursor, values: values })
  else
    let (next, value) = byte(cursor)?
    if value >= bound do
      return Err("SOLANA_TRANSACTION: account index out of bounds")
    end
    indexes(next, count - 1, bound, List.append(values, value))
  end
end

struct Instructions do
  cursor :: Cursor
  values :: List<Instruction>
end

fn instructions(cursor :: Cursor,
  count :: Int,
  accounts :: List<AccountMeta>,
  values :: List<Instruction>) -> Instructions!String do
  if count == 0 do
    Ok(Instructions { cursor: cursor, values: values })
  else
    let (with_program, program) = byte(cursor)?
    if program >= List.length(accounts) do
      return Err("SOLANA_TRANSACTION: program index out of bounds")
    end
    let (with_count, account_count) = compact(with_program)?
    if account_count > 64 do
      return Err("SOLANA_TRANSACTION: too many instruction accounts")
    end
    let refs = indexes(with_count, account_count, List.length(accounts), [])?
    let (with_length, length) = compact(refs.cursor)?
    if length > 1232 do
      return Err("SOLANA_TRANSACTION: instruction data too large")
    end
    let (next_cursor, data) = fixed(with_length, length)?
    let metas = for index in refs.values do
      List.get(accounts, index)
    end
    let instruction = Instruction {
      program_id: List.get(accounts, program).pubkey,
      accounts: metas,
      data: data
    }
    instructions(next_cursor, count - 1, accounts, List.append(values, instruction))
  end
end

# Authenticate every required signature against the exact full message bytes.
# This checks signatures, framing and signed account roles, not chain finality.
pub fn signed_transaction(wire :: Bytes, queried_signature :: Bytes) -> SignedTransaction!String do
  if Bytes.length(wire) > 1232 || Bytes.length(queried_signature) != 64 do
    return Err("SOLANA_TRANSACTION: invalid wire/signature length")
  end
  let (with_count, count) = compact(Cursor { bytes: wire, offset: 0 })?
  if count < 1 || count > 12 do
    return Err("SOLANA_TRANSACTION: invalid signature count")
  end
  let signature_vector = fixed_vector(with_count, count, 64, [])?
  if !Bytes.secure_equals(List.get(signature_vector.values, 0), queried_signature) do
    return Err("SOLANA_TRANSACTION: queried signature mismatch")
  end
  let message_start = signature_vector.cursor.offset
  let (with_first, first) = byte(signature_vector.cursor)?
  let version = if first == 128 do
    0
  else if first < 128 do
    -1
  else
    return Err("SOLANA_TRANSACTION: unsupported version")
  end
  let (after_required, required) = if version == 0 do
    byte(with_first)?
  else
    (with_first, first)
  end
  let (after_signed, readonly_signed) = byte(after_required)?
  let (after_unsigned, readonly_unsigned) = byte(after_signed)?
  let (with_keys, key_count) = compact(after_unsigned)?
  if key_count < 1
    || key_count > 64
    || required != count
    || readonly_signed >= required
    || required > key_count
    || readonly_unsigned > key_count - required do
    return Err("SOLANA_TRANSACTION: invalid message header")
  end
  let keys = fixed_vector(with_keys, key_count, 32, [])?
  let accounts = for index in 0..key_count do
    let signer = index < required
    AccountMeta {
      pubkey: Pubkey { bytes: List.get(keys.values, index) },
      signer: signer,
      writable: if signer do
        index < required - readonly_signed
      else
        index < key_count - readonly_unsigned
      end
    }
  end
  for index in 0..key_count do
    for earlier in 0..index do
      if pubkey_equal(List.get(accounts, index).pubkey, List.get(accounts, earlier).pubkey) do
        return Err("SOLANA_TRANSACTION: duplicate account key")
      end
    end
  end
  let (after_blockhash, blockhash) = fixed(keys.cursor, 32)?
  let (with_instructions, instruction_count) = compact(after_blockhash)?
  if instruction_count > 64 do
    return Err("SOLANA_TRANSACTION: too many instructions")
  end
  let decoded = instructions(with_instructions, instruction_count, accounts, [])?
  let end_cursor = if version == 0 do
    let (after_lookups, lookup_count) = compact(decoded.cursor)?
    if lookup_count != 0 do
      return Err("SOLANA_TRANSACTION: address lookup tables are unsupported")
    end
    after_lookups
  else
    decoded.cursor
  end
  if end_cursor.offset != Bytes.length(wire) do
    return Err("SOLANA_TRANSACTION: trailing bytes")
  end
  let message = Bytes.slice(wire, message_start, Bytes.length(wire) - message_start)?
  for index in 0..required do
    let valid = case Crypto.verify(SigningPublicKey { bytes: List.get(keys.values, index) },
      message,
      Signature { bytes: List.get(signature_vector.values, index) }) do
      Err(_) -> return Err("SOLANA_TRANSACTION: invalid signer/signature")
      Ok(value) -> value
    end
    if !valid do
      return Err("SOLANA_TRANSACTION: signature verification failed")
    end
  end
  Ok(SignedTransaction {
    signature: queried_signature,
    signatures: signature_vector.values,
    version: version,
    header: MessageHeader {
      num_required_signatures: required,
      num_readonly_signed_accounts: readonly_signed,
      num_readonly_unsigned_accounts: readonly_unsigned
    },
    accounts: accounts,
    instructions: decoded.values,
    message: message,
    recent_blockhash: blockhash,
    wire: wire
  })
end

fn uint(value :: Json) -> U64!String do
  U64.parse(Json.encode(value))
end

fn integer(value :: Json) -> Int!String do
  case String.to_int(Json.encode(value)) do
    None -> Err("SOLANA_TRANSACTION: expected integer metadata")
    Some(value) -> Ok(value)
  end
end

fn balances(root :: Json, name :: String, count :: Int) -> List<U64>!String do
  let values = Json.object_get(root, name)?
  if Json.array_length(values)? != count do
    return Err("SOLANA_TRANSACTION: balance count mismatch")
  end
  let output = for index in 0..count do
    uint(Json.array_get(values, index)?)?
  end
  Ok(output)
end

fn optional_key(root :: Json, name :: String) -> Option<Pubkey>!String do
  case Json.object_get(root, name) do
    Err(_) -> Ok(None)
    Ok(value) -> if Json.is_null(value) do
      Ok(None)
    else
      Ok(Some(pubkey(Json.as_string(value)?)?))
    end
  end
end

fn token_balances(root :: Json,
  name :: String,
  count :: Int) -> List<TransactionTokenBalance>!String do
  let values = Json.object_get(root, name)?
  let length = Json.array_length(values)?
  if length > count do
    return Err("SOLANA_TRANSACTION: too many token balances")
  end
  let output = for index in 0..length do
    let item = Json.array_get(values, index)?
    let account_index = integer(Json.object_get(item, "accountIndex")?)?
    if account_index < 0 || account_index >= count do
      return Err("SOLANA_TRANSACTION: token balance index out of bounds")
    end
    let token = Json.object_get(item, "uiTokenAmount")?
    let text = Json.as_string(Json.object_get(token, "amount")?)?
    let amount = U64.parse(text)?
    if U64.to_string(amount) != text do
      return Err("SOLANA_TRANSACTION: noncanonical token amount")
    end
    let decimals = integer(Json.object_get(token, "decimals")?)?
    if decimals < 0 || decimals > 255 do
      return Err("SOLANA_TRANSACTION: invalid token decimals")
    end
    TransactionTokenBalance {
      account_index: account_index,
      mint: pubkey(Json.as_string(Json.object_get(item, "mint")?)?)?,
      owner: optional_key(item, "owner")?,
      program_id: optional_key(item, "programId")?,
      amount: amount,
      decimals: decimals
    }
  end
  for index in 0..length do
    for earlier in 0..index do
      if List.get(output, index).account_index == List.get(output, earlier).account_index do
        return Err("SOLANA_TRANSACTION: duplicate token balance")
      end
    end
  end
  Ok(output)
end

fn inner_groups(value :: Json,
  accounts :: List<AccountMeta>,
  instruction_count :: Int) -> List<TransactionInnerGroup>!String do
  if Json.is_null(value) do
    return Ok([])
  end
  let count = Json.array_length(value)?
  if count > 64 do
    return Err("SOLANA_TRANSACTION: too many inner groups")
  end
  let groups = for index in 0..count do
    let item = Json.array_get(value, index)?
    let parent = integer(Json.object_get(item, "index")?)?
    if parent < 0 || parent >= instruction_count do
      return Err("SOLANA_TRANSACTION: inner parent out of bounds")
    end
    let rows = Json.object_get(item, "instructions")?
    let length = Json.array_length(rows)?
    if length > 64 do
      return Err("SOLANA_TRANSACTION: too many inner instructions")
    end
    let decoded = for row in 0..length do
      let instruction = Json.array_get(rows, row)?
      let program = integer(Json.object_get(instruction, "programIdIndex")?)?
      if program < 0 || program >= List.length(accounts) do
        return Err("SOLANA_TRANSACTION: inner program index out of bounds")
      end
      let refs = Json.object_get(instruction, "accounts")?
      let refs_count = Json.array_length(refs)?
      if refs_count > 64 do
        return Err("SOLANA_TRANSACTION: too many inner accounts")
      end
      let metas = for ref in 0..refs_count do
        let position = integer(Json.array_get(refs, ref)?)?
        if position < 0 || position >= List.length(accounts) do
          return Err("SOLANA_TRANSACTION: inner account index out of bounds")
        end
        List.get(accounts, position)
      end
      let encoded = Json.as_string(Json.object_get(instruction, "data")?)?
      let data = Bytes.from_base58(encoded)?
      if Bytes.length(data) > 1232 || Bytes.to_base58(data) != encoded do
        return Err("SOLANA_TRANSACTION: invalid inner data")
      end
      Instruction { program_id: List.get(accounts, program).pubkey, accounts: metas, data: data }
    end
    TransactionInnerGroup { instruction_index: parent, instructions: decoded }
  end
  for index in 0..count do
    for earlier in 0..index do
      if List.get(groups, index).instruction_index == List.get(groups, earlier).instruction_index do
        return Err("SOLANA_TRANSACTION: duplicate inner group")
      end
    end
  end
  Ok(groups)
end

pub fn get_finalized_transaction_request(id :: Int, signature :: Bytes) -> RpcRequest!String do
  if Bytes.length(signature) != 64 do
    return Err("SOLANA_TRANSACTION: invalid signature length")
  end
  rpc_request(id,
    "getTransaction",
    "[#{Json.encode_string(Bytes.to_base58(signature))},{\"encoding\":\"base64\",\"commitment\":\"finalized\",\"maxSupportedTransactionVersion\":0}]")
end

fn execution_error_name(value :: Json) -> String!String do
  let name = Json.as_string(value)?
  let bytes = Bytes.from_utf8(name)
  if Bytes.length(bytes) < 1 || Bytes.length(bytes) > 128 do
    return Err("SOLANA_TRANSACTION: invalid execution error name")
  end
  for index in 0..Bytes.length(bytes) do
    let byte = Bytes.get(bytes, index)?
    let letter = (byte >= 65 && byte <= 90) || (byte >= 97 && byte <= 122)
    if !letter && (index == 0 || byte < 48 || byte > 57) do
      return Err("SOLANA_TRANSACTION: invalid execution error name")
    end
  end
  Ok(Json.encode(value))
end

fn instruction_execution_error(value :: Json) -> String!String do
  case Json.as_string(value) do
    Ok(_) -> execution_error_name(value)
    Err(_) -> do
      let custom = uint(Json.object_get(value, "Custom")?)?
      let encoded = "{\"Custom\":" <> U64.to_string(custom) <> "}"
      if U64.compare(custom, U64.parse("4294967295")?) > 0 || Json.encode(value) != encoded do
        return Err("SOLANA_TRANSACTION: invalid custom instruction error")
      end
      Ok(encoded)
    end
  end
end

# Deliberately bounded: string TransactionError variants and InstructionError
# (string variant or u32 Custom). Unknown object variants fail closed.
fn transaction_execution_error(value :: Json, instruction_count :: Int) -> Option<String>!String do
  if Json.is_null(value) do
    return Ok(None)
  end
  if String.length(Json.encode(value)) > 2048 do
    return Err("SOLANA_TRANSACTION: oversized execution error")
  end
  case Json.as_string(value) do
    Ok(_) -> Ok(Some(execution_error_name(value)?))
    Err(_) -> do
      let error = Json.object_get(value, "InstructionError")?
      if Json.array_length(error)? != 2 do
        return Err("SOLANA_TRANSACTION: invalid instruction error")
      end
      let index = integer(Json.array_get(error, 0)?)?
      if index < 0 || index >= instruction_count do
        return Err("SOLANA_TRANSACTION: invalid failed instruction index")
      end
      let detail = instruction_execution_error(Json.array_get(error, 1)?)?
      let encoded = "{\"InstructionError\":[" <> Int.to_string(index) <> "," <> detail <> "]}"
      if Json.encode(value) != encoded do
        return Err("SOLANA_TRANSACTION: invalid execution error object")
      end
      Ok(Some(encoded))
    end
  end
end

pub fn finalized_transaction_outcome_from_response(response :: RpcResponse,
  signature :: Bytes,
  expected_id :: Int) -> Option<FinalizedTransactionOutcome>!String do
  if response.id != expected_id || !response.ok do
    return Err("SOLANA_TRANSACTION: mismatched or failed RPC response")
  end
  let root = Json.parse(response.result_json)?
  if Json.is_null(root) do
    return Ok(None)
  end
  let transaction = Json.object_get(root, "transaction")?
  if Json.array_length(transaction)? != 2
    || Json.as_string(Json.array_get(transaction, 1)?)? != "base64" do
    return Err("SOLANA_TRANSACTION: expected base64 transaction")
  end
  let encoded = Json.as_string(Json.array_get(transaction, 0)?)?
  let wire = Bytes.from_base64(encoded)?
  if Bytes.to_base64(wire) != encoded do
    return Err("SOLANA_TRANSACTION: noncanonical base64")
  end
  let signed = signed_transaction(wire, signature)?
  let version = Json.object_get(root, "version")?
  if (signed.version == -1 && Json.encode(version) != "\"legacy\"")
    || (signed.version == 0 && Json.encode(version) != "0") do
    return Err("SOLANA_TRANSACTION: RPC version mismatch")
  end
  let meta = Json.object_get(root, "meta")?
  let execution_error = transaction_execution_error(Json.object_get(meta, "err")?,
    List.length(signed.instructions))?
  let loaded = Json.object_get(meta, "loadedAddresses")?
  if Json.array_length(Json.object_get(loaded, "writable")?)? != 0
    || Json.array_length(Json.object_get(loaded, "readonly")?)? != 0 do
    return Err("SOLANA_TRANSACTION: unexpected loaded addresses")
  end
  let inner = Json.object_get(meta, "innerInstructions")?
  let inner_count = if Json.is_null(inner) do
    -1
  else
    Json.array_length(inner)?
  end
  if inner_count > 64 do
    return Err("SOLANA_TRANSACTION: too many inner instruction groups")
  end
  let block_time_value = Json.object_get(root, "blockTime")?
  let block_time = if Json.is_null(block_time_value) do
    None
  else
    Some(integer(block_time_value)?)
  end
  let count = List.length(signed.accounts)
  Ok(Some(FinalizedTransactionOutcome {
    transaction: FinalizedTransaction {
      signed: signed,
      slot: uint(Json.object_get(root, "slot")?)?,
      block_time: block_time,
      fee: uint(Json.object_get(meta, "fee")?)?,
      pre_balances: balances(meta, "preBalances", count)?,
      post_balances: balances(meta, "postBalances", count)?,
      pre_token_balances: token_balances(meta, "preTokenBalances", count)?,
      post_token_balances: token_balances(meta, "postTokenBalances", count)?,
      inner_instruction_count: inner_count,
      inner_instructions: inner_groups(inner, signed.accounts, List.length(signed.instructions))?
    },
    execution_error: execution_error
  }))
end

pub fn finalized_transaction_from_response(response :: RpcResponse,
  signature :: Bytes,
  expected_id :: Int) -> Option<FinalizedTransaction>!String do
  case finalized_transaction_outcome_from_response(response, signature, expected_id)? do
    None -> Ok(None)
    Some(value) -> case value.execution_error do
      Some(_) -> Err("SOLANA_TRANSACTION: transaction failed")
      None -> Ok(Some(value.transaction))
    end
  end
end

# Finality and execution metadata are assertions by the configured RPC provider.
# Signatures authenticate the exact wire message, not the RPC's chain history.
pub fn get_finalized_transaction(client :: Int,
  url :: String,
  signature :: Bytes,
  id :: Int,
  timeout_ms :: Int,
  max_response_bytes :: Int) -> Option<FinalizedTransaction>!String do
  let response = rpc_send(client,
    url,
    get_finalized_transaction_request(id, signature)?,
    timeout_ms,
    max_response_bytes)?
  finalized_transaction_from_response(response, signature, id)
end

# Failure is authenticated to the same signed wire and provider context as
# success. A caller must inspect execution_error before recording an effect.
pub fn get_finalized_transaction_outcome(client :: Int,
  url :: String,
  signature :: Bytes,
  id :: Int,
  timeout_ms :: Int,
  max_response_bytes :: Int) -> Option<FinalizedTransactionOutcome>!String do
  let response = rpc_send(client,
    url,
    get_finalized_transaction_request(id, signature)?,
    timeout_ms,
    max_response_bytes)?
  finalized_transaction_outcome_from_response(response, signature, id)
end
