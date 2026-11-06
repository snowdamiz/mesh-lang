pub struct Pubkey do
  bytes :: Bytes
end

pub struct Signature do
  bytes :: Bytes
end

pub struct Hash do
  bytes :: Bytes
end

pub struct Slot do
  value :: U64
end

pub struct BlockHeight do
  value :: U64
end

pub struct EpochInfo do
  epoch :: U64
  absolute_slot :: U64
end

pub struct LatestBlockhash do
  context_slot :: U64
  blockhash :: Hash
  last_valid_block_height :: U64
end

pub struct RpcRequest do
  id :: Int
  method :: String
  params_json :: String
end

pub struct RpcResponse do
  id :: Int
  ok :: Bool
  result_json :: String
  error_json :: String
end

pub struct AccountInfo do
  data :: Bytes
  executable :: Bool
  lamports :: U64
  owner :: Pubkey
  rent_epoch :: U64
end

pub struct AccountsAtSlot do
  slot :: U64
  accounts :: List<AccountInfo>
end

pub struct OptionalAccountsAtSlot do
  slot :: U64
  accounts :: List<Option<AccountInfo>>
end

pub struct KeyedAccountInfo do
  address :: Pubkey
  account :: AccountInfo
end

pub struct ProgramAccountsAtSlot do
  slot :: U64
  accounts :: List<KeyedAccountInfo>
end

pub struct AccountAtSlot do
  slot :: U64
  account :: AccountInfo
end

## A data slice plus the full account allocation reported by RPC.
pub struct AccountSlice do
  info :: AccountInfo
  space :: U64
end

pub struct AccountSliceAtSlot do
  slot :: U64
  account :: Option<AccountSlice>
end

pub struct OptionalAccountSlicesAtSlot do
  slot :: U64
  accounts :: List<Option<AccountSlice>>
end

pub struct SolanaClock do
  slot :: U64
  epoch_start_timestamp :: Int
  epoch :: U64
  leader_schedule_epoch :: U64
  unix_timestamp :: Int
end

pub struct TokenAccount do
  mint :: Pubkey
  owner :: Pubkey
  amount :: U64
  delegate :: Option<Pubkey>
  state :: Int
  native_reserve :: Option<U64>
  delegated_amount :: U64
  close_authority :: Option<Pubkey>
end

pub struct Mint do
  mint_authority :: Option<Pubkey>
  supply :: U64
  decimals :: Int
  initialized :: Bool
  freeze_authority :: Option<Pubkey>
end

pub struct StakePoolState do
  account_type :: Int
  total_lamports :: U64
  pool_token_supply :: U64
  last_update_epoch :: U64
end

pub struct ProgramAccountFilter do
  encoded :: String
end

pub struct AccountNotification do
  subscription :: Int
  slot :: U64
  account :: AccountInfo
end

pub struct SlotNotification do
  subscription :: Int
  slot :: U64
  parent :: U64
  root :: U64
end

fn fixed_base58(value :: String, length :: Int, label :: String) -> Bytes!String do
  case value
    |> Bytes.from_base58() do
    Err(_) -> Err("SOLANA_#{label}: invalid base58")
    Ok(bytes) -> if Bytes.length(bytes) != length do
      Err("SOLANA_#{label}: expected #{length} bytes, got #{Bytes.length(bytes)}")
    else if (bytes
      |> Bytes.to_base58()) == value do
      Ok(bytes)
    else
      Err("SOLANA_#{label}: non-canonical base58")
    end
  end
end

fn pubkey_from_bytes(bytes :: Bytes) -> Pubkey!String do
  if Bytes.length(bytes) == 32 do
    Ok(Pubkey { bytes: bytes })
  else
    Err("SOLANA_PUBKEY: expected 32 bytes, got #{Bytes.length(bytes)}")
  end
end

pub fn pubkey(value :: String) -> Pubkey!String do
  Ok(Pubkey {
    bytes: (value
      |> fixed_base58(32, "PUBKEY"))?
  })
end

pub fn signature(value :: String) -> Signature!String do
  Ok(Signature {
    bytes: (value
      |> fixed_base58(64, "SIGNATURE"))?
  })
end

pub fn hash_value(value :: String) -> Hash!String do
  Ok(Hash {
    bytes: (value
      |> fixed_base58(32, "HASH"))?
  })
end

pub fn pubkey_string(value :: Pubkey) -> String do
  value.bytes
    |> Bytes.to_base58()
end

pub fn signature_string(value :: Signature) -> String do
  value.bytes
    |> Bytes.to_base58()
end

pub fn hash_string(value :: Hash) -> String do
  value.bytes
    |> Bytes.to_base58()
end

pub fn pubkey_equal(left :: Pubkey, right :: Pubkey) -> Bool do
  left.bytes
    |> Bytes.secure_equals(right.bytes)
end

pub fn slot(value :: String) -> Slot!String do
  Ok(Slot {
    value: (value
      |> U64.parse())?
  })
end

pub fn block_height(value :: String) -> BlockHeight!String do
  Ok(BlockHeight {
    value: (value
      |> U64.parse())?
  })
end

pub fn spl_token_program() -> Pubkey!String do
  "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
    |> pubkey()
end

pub fn spl_stake_pool_program() -> Pubkey!String do
  "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy"
    |> pubkey()
end

pub fn jitosol_stake_pool() -> Pubkey!String do
  "Jito4APyf642JPZPx3hGc6WWJ8zPKtRbRs4P815Awbb"
    |> pubkey()
end

pub fn jitosol_mint() -> Pubkey!String do
  "J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn"
    |> pubkey()
end

fn u64_text(value :: String, label :: String) -> U64!String do
  case U64.parse(value) do
    Ok(parsed)
    Err(_) -> Err("SOLANA_#{label}: expected an unsigned integer")
  end
end

fn u64_field(raw :: String, field :: String, label :: String) -> U64!String do
  let root = Json.parse(raw)?
  let value = case Json.object_get(root, field) do
    Err(_) -> return Err("SOLANA_#{label}: missing field #{field}")
    Ok(value) -> value
  end
  # Preserve the JSON type: a quoted number is not RPC integer metadata.
  u64_text(Json.encode(value), label)
end

fn int_field(raw :: String, field :: String, label :: String) -> Int!String do
  case raw
    |> Json.get(field)
    |> String.to_int() do
    Some(parsed) -> if parsed >= 0 do
      Ok(parsed)
    else
      Err("SOLANA_#{label}: field #{field} must be non-negative")
    end
    None -> Err("SOLANA_#{label}: field #{field} must be an integer")
  end
end

fn account_data(encoded :: String) -> Bytes!String do
  case encoded
    |> Bytes.from_base64() do
    Err(_) -> Err("SOLANA_ACCOUNT: invalid base64 data")
    Ok(decoded) -> if (decoded
      |> Bytes.to_base64()) == encoded do
      Ok(decoded)
    else
      Err("SOLANA_ACCOUNT: non-canonical base64 data")
    end
  end
end

pub fn account_info(raw :: String) -> AccountInfo!String do
  let root = (raw
    |> Json.parse())?
  let data = (root
    |> Json.object_get("data"))?
  if (data
    |> Json.array_length())? != 2 do
    Err("SOLANA_ACCOUNT: data must be [payload, encoding]")
  else
    let encoded = ((data
      |> Json.array_get(0))?
      |> Json.as_string())?
    let encoding = ((data
      |> Json.array_get(1))?
      |> Json.as_string())?
    if encoding != "base64" do
      Err("SOLANA_ACCOUNT: data encoding must be base64")
    else
      let owner_text = Json.as_string(Json.object_get(root, "owner")?)?
      if owner_text == "" do
        Err("SOLANA_ACCOUNT: missing owner")
      else
        Ok(AccountInfo {
          data: (encoded
            |> account_data())?,
          executable: ((root
            |> Json.object_get("executable"))?
            |> Json.as_bool())?,
          lamports: (raw
            |> u64_field("lamports", "ACCOUNT"))?,
          owner: (owner_text
            |> pubkey())?,
          rent_epoch: (raw
            |> u64_field("rentEpoch", "ACCOUNT"))?
        })
      end
    end
  end
end

fn read_u64(data :: Bytes, offset :: Int, label :: String) -> U64!String do
  case data
    |> Bytes.read_uint_le(offset, 8) do
    Ok(value) -> value
      |> u64_text(label)
    Err(_) -> Err("SOLANA_#{label}: u64 read is out of bounds")
  end
end

pub fn clock_sysvar_address() -> Pubkey!String do
  pubkey("SysvarC1ock11111111111111111111111111111111")
end

fn clock_timestamp(data :: Bytes, offset :: Int) -> Int!String do
  let number = read_u64(data, offset, "CLOCK")?
  if U64.compare(number, U64.parse("9223372036854775807")?) <= 0 do
    U64.to_int(number)
  else
    I128.to_int(I128.subtract(I128.parse(U64.to_string(number))?,
      I128.parse("18446744073709551616")?)?)
  end
end

## Decode the standard 40-byte Clock sysvar, including negative i64 timestamps.
## The caller authenticates the requested address, RPC context and provider.
pub fn clock_sysvar(account :: AccountInfo) -> SolanaClock!String do
  if !pubkey_equal(account.owner, pubkey("Sysvar1111111111111111111111111111111111111")?)
    || account.executable
    || account.lamports == U64.parse("0")?
    || Bytes.length(account.data) != 40 do
    return Err("SOLANA_CLOCK: invalid sysvar account")
  end
  Ok(SolanaClock {
    slot: read_u64(account.data, 0, "CLOCK")?,
    epoch_start_timestamp: clock_timestamp(account.data, 8)?,
    epoch: read_u64(account.data, 16, "CLOCK")?,
    leader_schedule_epoch: read_u64(account.data, 24, "CLOCK")?,
    unix_timestamp: clock_timestamp(account.data, 32)?
  })
end

fn read_byte(data :: Bytes, offset :: Int, label :: String) -> Int!String do
  case Bytes.get(data, offset) do
    Ok(value)
    Err(_) -> Err("SOLANA_#{label}: byte read is out of bounds")
  end
end

fn coption_pubkey(data :: Bytes, offset :: Int, label :: String) -> Option<Pubkey>!String do
  case Bytes.read_uint_le(data, offset, 4) do
    Ok("0") -> Ok(None)
    Ok("1") -> case Bytes.slice(data, offset + 4, 32) do
      Err(_) -> Err("SOLANA_#{label}: COption read is out of bounds")
      Ok(bytes) -> Ok(Some(pubkey_from_bytes(bytes)?))
    end
    Ok(tag) -> Err("SOLANA_#{label}: invalid COption tag #{tag}")
    Err(_) -> Err("SOLANA_#{label}: COption read is out of bounds")
  end
end

fn coption_u64(data :: Bytes, offset :: Int, label :: String) -> Option<U64>!String do
  case data
    |> Bytes.read_uint_le(offset, 4) do
    Ok("0") -> Ok(None)
    Ok("1") -> Ok(Some((data
      |> read_u64(offset + 4, label))?))
    Ok(tag) -> Err("SOLANA_#{label}: invalid COption tag #{tag}")
    Err(_) -> Err("SOLANA_#{label}: COption read is out of bounds")
  end
end

fn validate_owner(account :: AccountInfo, expected :: Pubkey, label :: String) -> Int!String do
  if account.owner
    |> pubkey_equal(expected) do
    Ok(0)
  else
    Err("SOLANA_#{label}: account owner mismatch")
  end
end

pub fn token_account(account :: AccountInfo) -> TokenAccount!String do
  (account
    |> validate_owner((spl_token_program())?, "TOKEN_ACCOUNT"))?
  if Bytes.length(account.data) != 165 do
    Err("SOLANA_TOKEN_ACCOUNT: expected 165 bytes, got #{Bytes.length(account.data)}")
  else
    let state = (account.data
      |> read_byte(108, "TOKEN_ACCOUNT"))?
    if state < 0 || state > 2 do
      Err("SOLANA_TOKEN_ACCOUNT: invalid state #{state}")
    else
      Ok(TokenAccount {
        mint: ((account.data
          |> Bytes.slice(0, 32))?
          |> pubkey_from_bytes())?,
        owner: ((account.data
          |> Bytes.slice(32, 32))?
          |> pubkey_from_bytes())?,
        amount: (account.data
          |> read_u64(64, "TOKEN_ACCOUNT"))?,
        delegate: (account.data
          |> coption_pubkey(72, "TOKEN_ACCOUNT"))?,
        state: state,
        native_reserve: (account.data
          |> coption_u64(109, "TOKEN_ACCOUNT"))?,
        delegated_amount: (account.data
          |> read_u64(121, "TOKEN_ACCOUNT"))?,
        close_authority: (account.data
          |> coption_pubkey(129, "TOKEN_ACCOUNT"))?
      })
    end
  end
end

pub fn mint(account :: AccountInfo) -> Mint!String do
  (account
    |> validate_owner((spl_token_program())?, "MINT"))?
  if Bytes.length(account.data) != 82 do
    Err("SOLANA_MINT: expected 82 bytes, got #{Bytes.length(account.data)}")
  else
    let initialized = (account.data
      |> read_byte(45, "MINT"))?
    if initialized != 0 && initialized != 1 do
      Err("SOLANA_MINT: initialized flag must be 0 or 1")
    else
      Ok(Mint {
        mint_authority: (account.data
          |> coption_pubkey(0, "MINT"))?,
        supply: (account.data
          |> read_u64(36, "MINT"))?,
        decimals: (account.data
          |> read_byte(44, "MINT"))?,
        initialized: initialized == 1,
        freeze_authority: (account.data
          |> coption_pubkey(46, "MINT"))?
      })
    end
  end
end

pub fn stake_pool(account :: AccountInfo) -> StakePoolState!String do
  (account
    |> validate_owner((spl_stake_pool_program())?, "STAKE_POOL"))?
  if Bytes.length(account.data) != 611 do
    Err("SOLANA_STAKE_POOL: expected 611 bytes, got #{Bytes.length(account.data)}")
  else
    let account_type = (account.data
      |> read_byte(0, "STAKE_POOL"))?
    if account_type != 1 do
      Err("SOLANA_STAKE_POOL: unsupported account type #{account_type}")
    else
      Ok(StakePoolState {
        account_type: account_type,
        total_lamports: (account.data
          |> read_u64(258, "STAKE_POOL"))?,
        pool_token_supply: (account.data
          |> read_u64(266, "STAKE_POOL"))?,
        last_update_epoch: (account.data
          |> read_u64(274, "STAKE_POOL"))?
      })
    end
  end
end

fn validate_jitosol(pool_address :: Pubkey,
  pool :: StakePoolState,
  mint_address :: Pubkey,
  mint_state :: Mint,
  current_epoch :: U64) -> Int!String do
  if !pubkey_equal(pool_address, (jitosol_stake_pool())?) do
    Err("SOLANA_JITOSOL: unexpected stake-pool address")
  else if !pubkey_equal(mint_address, (jitosol_mint())?) do
    Err("SOLANA_JITOSOL: unexpected mint address")
  else if U64.compare(pool.total_lamports, (U64.parse("0"))?) <= 0 do
    Err("SOLANA_JITOSOL: total pool lamports must be positive")
  else if U64.compare(pool.pool_token_supply, (U64.parse("0"))?) <= 0 do
    Err("SOLANA_JITOSOL: pool token supply must be positive")
  else if U64.compare(mint_state.supply, pool.pool_token_supply) > 0 do
    Err("SOLANA_JITOSOL: mint supply exceeds stake pool accounting")
  else if U64.compare(pool.last_update_epoch, current_epoch) != 0 do
    Err("SOLANA_JITOSOL: stake pool is stale for the current epoch")
  else if mint_state.decimals != 9 || !mint_state.initialized do
    Err("SOLANA_JITOSOL: unexpected mint configuration")
  else
    Ok(0)
  end
end

pub fn jitosol_nav(pool_address :: Pubkey,
  pool :: StakePoolState,
  mint_address :: Pubkey,
  mint_state :: Mint,
  current_epoch :: U64) -> U128!String do
  (validate_jitosol(pool_address, pool, mint_address, mint_state, current_epoch))?
  let total = (pool.total_lamports
    |> U64.to_string()
    |> U128.parse())?
  let supply = (pool.pool_token_supply
    |> U64.to_string()
    |> U128.parse())?
  ((total
    |> U128.multiply((U128.parse("1000000000"))?))?
    |> U128.divide(supply))
end

fn validate_commitment(value :: String) -> Int!String do
  if value == "processed" || value == "confirmed" || value == "finalized" do
    Ok(0)
  else
    Err("SOLANA_RPC: unsupported commitment #{value}")
  end
end

pub fn rpc_request(id :: Int, method :: String, params_json :: String) -> RpcRequest!String do
  if id < 0 do
    Err("SOLANA_RPC: request id must be non-negative")
  else if String.length(method) == 0 do
    Err("SOLANA_RPC: method must not be empty")
  else
    case params_json
      |> Json.parse() do
      Ok(_) -> Ok(RpcRequest { id: id, method: method, params_json: params_json })
      Err(_) -> Err("SOLANA_RPC: params must be valid JSON")
    end
  end
end

pub fn rpc_request_json(request :: RpcRequest) -> String do
  """{"jsonrpc":"2.0","id":#{request.id},"method":#{Json.encode_string(request.method)},"params":#{request.params_json}}"""
end

fn account_params(address :: Pubkey, commitment :: String) -> String!String do
  (commitment
    |> validate_commitment())?
  Ok("[#{Json.encode_string(pubkey_string(address))},{\"commitment\":#{Json.encode_string(commitment)},\"encoding\":\"base64\"}]")
end

pub fn get_account_info_request(id :: Int,
  address :: Pubkey,
  commitment :: String) -> RpcRequest!String do
  rpc_request(id,
    "getAccountInfo",
    (address
      |> account_params(commitment))?)
end

## Read one bounded byte range while preserving a minimum RPC context slot.
## The caller still verifies the returned owner, context and exact slice length.
pub fn get_account_info_slice_request(id :: Int,
  address :: Pubkey,
  commitment :: String,
  offset :: Int,
  length :: Int,
  min_context_slot :: Option<U64>) -> RpcRequest!String do
  validate_commitment(commitment)?
  if Bytes.length(address.bytes) != 32
    || offset < 0
    || length < 0
    || offset > 10485760
    || length > 10485760 - offset do
    return Err("SOLANA_RPC: invalid account slice")
  end
  let minimum = case min_context_slot do
    None -> ""
    Some(slot) -> ",\"minContextSlot\":#{U64.to_string(slot)}"
  end
  rpc_request(id,
    "getAccountInfo",
    "[#{Json.encode_string(pubkey_string(address))},{\"commitment\":#{Json.encode_string(commitment)},\"encoding\":\"base64\",\"dataSlice\":{\"offset\":#{offset},\"length\":#{length}}#{minimum}}]")
end

fn pubkeys_json_loop(values :: List<Pubkey>,
  index :: Int,
  total :: Int,
  output :: String) -> String do
  if index >= total do
    output <> "]"
  else
    let separator = if index == 0 do
      ""
    else
      ","
    end
    pubkeys_json_loop(values,
      index + 1,
      total,
      output <> separator <> Json.encode_string(pubkey_string(List.get(values, index))))
  end
end

fn pubkeys_json(values :: List<Pubkey>) -> String do
  pubkeys_json_loop(values, 0, List.length(values), "[")
end

pub fn get_multiple_accounts_request(id :: Int,
  addresses :: List<Pubkey>,
  commitment :: String) -> RpcRequest!String do
  get_multiple_accounts_context_request(id, addresses, commitment, None)
end

## Read an ordered batch from one bank, optionally requiring a minimum context.
## Providers may return null for missing accounts. Callers still authenticate
## ownership, response ID, account count, minimum context and provider trust.
pub fn get_multiple_accounts_context_request(id :: Int,
  addresses :: List<Pubkey>,
  commitment :: String,
  min_context_slot :: Option<U64>) -> RpcRequest!String do
  validate_commitment(commitment)?
  if List.length(addresses) < 1 || List.length(addresses) > 100 do
    return Err("SOLANA_RPC: getMultipleAccounts requires 1 to 100 addresses")
  end
  for address in addresses do
    if Bytes.length(address.bytes) != 32 do
      return Err("SOLANA_RPC: invalid account address")
    end
  end
  let minimum = case min_context_slot do
    None -> ""
    Some(slot) -> ",\"minContextSlot\":#{U64.to_string(slot)}"
  end
  rpc_request(id,
    "getMultipleAccounts",
    "[#{pubkeys_json(addresses)},{\"commitment\":#{Json.encode_string(commitment)},\"encoding\":\"base64\"#{minimum}}]")
end

## Read the same bounded byte range from an ordered batch in one bank.
## Full account space is retained by optional_accounts_slice_from_response.
pub fn get_multiple_accounts_slice_request(id :: Int,
  addresses :: List<Pubkey>,
  commitment :: String,
  offset :: Int,
  length :: Int,
  min_context_slot :: Option<U64>) -> RpcRequest!String do
  let request = get_multiple_accounts_context_request(id, addresses, commitment, min_context_slot)?
  if offset < 0 || length < 0 || offset > 10485760 || length > 10485760 - offset do
    return Err("SOLANA_RPC: invalid account slice")
  end
  let minimum = case min_context_slot do
    None -> ""
    Some(slot) -> ",\"minContextSlot\":#{U64.to_string(slot)}"
  end
  Ok(%{request |
    params_json: "[#{pubkeys_json(addresses)},{\"commitment\":#{Json.encode_string(commitment)},\"encoding\":\"base64\",\"dataSlice\":{\"offset\":#{offset},\"length\":#{length}}#{minimum}}]"
  })
end

fn commitment_params(commitment :: String) -> String!String do
  (commitment
    |> validate_commitment())?
  Ok("[{\"commitment\":#{Json.encode_string(commitment)}}]")
end

pub fn get_slot_request(id :: Int, commitment :: String) -> RpcRequest!String do
  rpc_request(id,
    "getSlot",
    (commitment
      |> commitment_params())?)
end

pub fn get_block_height_request(id :: Int, commitment :: String) -> RpcRequest!String do
  rpc_request(id,
    "getBlockHeight",
    (commitment
      |> commitment_params())?)
end

# getBlockTime takes a slot only; callers establish finality separately.
pub fn get_block_time_request(id :: Int, slot :: U64) -> RpcRequest!String do
  rpc_request(id, "getBlockTime", "[#{U64.to_string(slot)}]")
end

pub fn get_epoch_info_request(id :: Int, commitment :: String) -> RpcRequest!String do
  rpc_request(id,
    "getEpochInfo",
    (commitment
      |> commitment_params())?)
end

pub fn get_latest_blockhash_request(id :: Int, commitment :: String) -> RpcRequest!String do
  rpc_request(id,
    "getLatestBlockhash",
    (commitment
      |> commitment_params())?)
end

pub fn memcmp_filter(offset :: Int, bytes :: String) -> ProgramAccountFilter!String do
  if offset < 0 do
    Err("SOLANA_FILTER: memcmp offset must be non-negative")
  else
    case bytes
      |> Bytes.from_base58() do
      Err(_) -> Err("SOLANA_FILTER: memcmp bytes must be base58")
      Ok(decoded) -> if Bytes.length(decoded) > 128 do
        Err("SOLANA_FILTER: memcmp bytes exceed 128 decoded bytes")
      else
        Ok(ProgramAccountFilter {
          encoded: "{\"memcmp\":{\"offset\":#{offset},\"bytes\":#{Json.encode_string(bytes)}}}"
        })
      end
    end
  end
end

pub fn data_size_filter(size :: Int) -> ProgramAccountFilter!String do
  if size < 0 do
    Err("SOLANA_FILTER: data size must be non-negative")
  else
    Ok(ProgramAccountFilter { encoded: "{\"dataSize\":#{size}}" })
  end
end

fn filters_json_loop(values :: List<ProgramAccountFilter>,
  index :: Int,
  total :: Int,
  output :: String) -> String do
  if index >= total do
    output <> "]"
  else
    let separator = if index == 0 do
      ""
    else
      ","
    end
    let filter = List.get(values, index)
    filters_json_loop(values, index + 1, total, output <> separator <> filter.encoded)
  end
end

pub fn filters_json(values :: List<ProgramAccountFilter>) -> String do
  filters_json_loop(values, 0, List.length(values), "[")
end

pub fn program_accounts_request(id :: Int,
  program :: Pubkey,
  filters :: List<ProgramAccountFilter>,
  commitment :: String) -> RpcRequest!String do
  (commitment
    |> validate_commitment())?
  rpc_request(id,
    "getProgramAccounts",
    "[#{Json.encode_string(pubkey_string(program))},{\"commitment\":#{Json.encode_string(commitment)},\"encoding\":\"base64\",\"filters\":#{filters_json(filters)}}]")
end

## Discover keyed program accounts with a finalized-bank context. Callers must
## still check response IDs, minimum context, owners, filters and provider trust.
pub fn program_accounts_context_request(id :: Int,
  program :: Pubkey,
  filters :: List<ProgramAccountFilter>,
  commitment :: String,
  min_context_slot :: Option<U64>) -> RpcRequest!String do
  validate_commitment(commitment)?
  if Bytes.length(program.bytes) != 32 do
    return Err("SOLANA_RPC: invalid program address")
  end
  let minimum = case min_context_slot do
    None -> ""
    Some(value) -> ",\"minContextSlot\":" <> U64.to_string(value)
  end
  rpc_request(id,
    "getProgramAccounts",
    "[#{Json.encode_string(pubkey_string(program))},{\"commitment\":#{Json.encode_string(commitment)},\"encoding\":\"base64\",\"filters\":#{filters_json(filters)},\"withContext\":true#{minimum}}]")
end

pub fn account_subscribe_request(id :: Int,
  address :: Pubkey,
  commitment :: String) -> RpcRequest!String do
  rpc_request(id,
    "accountSubscribe",
    (address
      |> account_params(commitment))?)
end

pub fn slot_subscribe_request(id :: Int) -> RpcRequest do
  RpcRequest { id: id, method: "slotSubscribe", params_json: "[]" }
end

pub fn program_subscribe_request(id :: Int,
  program :: Pubkey,
  filters :: List<ProgramAccountFilter>,
  commitment :: String) -> RpcRequest!String do
  (commitment
    |> validate_commitment())?
  rpc_request(id,
    "programSubscribe",
    "[#{Json.encode_string(pubkey_string(program))},{\"commitment\":#{Json.encode_string(commitment)},\"encoding\":\"base64\",\"filters\":#{filters_json(filters)}}]")
end

pub fn send_account_subscription(connection :: Int,
  id :: Int,
  address :: Pubkey,
  commitment :: String) do
  ((id
    |> account_subscribe_request(address, commitment))?
    |> rpc_request_json() |2> WsClient.send_text(connection))
end

pub fn send_slot_subscription(connection :: Int, id :: Int) do
  (id
    |> slot_subscribe_request()
    |> rpc_request_json() |2> WsClient.send_text(connection))
end

pub fn send_program_subscription(connection :: Int,
  id :: Int,
  program :: Pubkey,
  filters :: List<ProgramAccountFilter>,
  commitment :: String) do
  ((id
    |> program_subscribe_request(program, filters, commitment))?
    |> rpc_request_json() |2> WsClient.send_text(connection))
end

pub fn rpc_response(raw :: String) -> RpcResponse!String do
  let parsed = Json.parse(raw)?
  if Json.as_string(Json.object_get(parsed, "jsonrpc")?)? != "2.0" do
    Err("SOLANA_RPC: response jsonrpc must be 2.0")
  else
    let id = case String.to_int(Json.encode(Json.object_get(parsed, "id")?)) do
      Some(value) when value >= 0 -> value
      _ -> return Err("SOLANA_RPC: response id must be a non-negative integer")
    end
    # Preserve the JSON type of a result. Json.get unwraps strings, which
    # otherwise makes a quoted number indistinguishable from an integer.
    let error = case Json.object_get(parsed, "error") do
      Err(_) -> ""
      Ok(value) -> Json.encode(value)
    end
    let result = case Json.object_get(parsed, "result") do
      Err(_) -> ""
      Ok(value) -> Json.encode(value)
    end
    if error != "" && error != "null" do
      if result != "" do
        Err("SOLANA_RPC: response has both result and error")
      else
        Ok(RpcResponse { id: id, ok: false, result_json: "", error_json: error })
      end
    else if result == "" do
      Err("SOLANA_RPC: response has neither result nor error")
    else
      Ok(RpcResponse { id: id, ok: true, result_json: result, error_json: "" })
    end
  end
end

fn require_rpc_result(response :: RpcResponse) -> String!String do
  if response.ok do
    Ok(response.result_json)
  else
    Err("SOLANA_RPC: #{response.error_json}")
  end
end

pub fn account_from_response(response :: RpcResponse) -> AccountAtSlot!String do
  let result = Json.parse(require_rpc_result(response)?)?
  let context = Json.object_get(result, "context")?
  let slot = u64_text(Json.encode(Json.object_get(context, "slot")?), "RPC")?
  let value = Json.object_get(result, "value")?
  if Json.is_null(value) do
    return Err("SOLANA_RPC: account was not found")
  end
  Ok(AccountAtSlot { slot: slot, account: account_info(Json.encode(value))? })
end

fn account_slice(value :: Json) -> AccountSlice!String do
  let info = account_info(Json.encode(value))?
  let space = u64_text(Json.encode(Json.object_get(value, "space")?), "ACCOUNT_SPACE")?
  if U64.compare(space, U64.parse(Int.to_string(Bytes.length(info.data)))?) < 0 do
    return Err("SOLANA_ACCOUNT: data exceeds full account space")
  end
  Ok(AccountSlice { info: info, space: space })
end

## Preserve absence and full allocation while decoding only the returned bytes.
## Callers bind the response to requested addresses, slice bounds and trust.
pub fn account_slice_from_response(response :: RpcResponse) -> AccountSliceAtSlot!String do
  let result = Json.parse(require_rpc_result(response)?)?
  let context = Json.object_get(result, "context")?
  let slot = u64_text(Json.encode(Json.object_get(context, "slot")?), "RPC")?
  let value = Json.object_get(result, "value")?
  let account = if Json.is_null(value) do
    None
  else
    Some(account_slice(value)?)
  end
  Ok(AccountSliceAtSlot { slot: slot, account: account })
end

pub fn slot_from_response(response :: RpcResponse) -> Slot!String do
  Ok(Slot {
    value: ((response
      |> require_rpc_result())?
      |> u64_text("SLOT"))?
  })
end

pub fn block_height_from_response(response :: RpcResponse) -> BlockHeight!String do
  Ok(BlockHeight {
    value: ((response
      |> require_rpc_result())?
      |> u64_text("BLOCK_HEIGHT"))?
  })
end

# Unknown/pruned time is None, not the Unix epoch. Errors remain errors.
pub fn block_time_from_response(response :: RpcResponse) -> Option<Int>!String do
  let result = require_rpc_result(response)?
  if result == "null" do
    Ok(None)
  else
    case String.to_int(result) do
      Some(value) -> Ok(Some(value))
      None -> Err("SOLANA_BLOCK_TIME: expected signed integer seconds or null")
    end
  end
end

pub fn epoch_info_from_response(response :: RpcResponse) -> EpochInfo!String do
  let result = (response
    |> require_rpc_result())?
  Ok(EpochInfo {
    epoch: (result
      |> u64_field("epoch", "EPOCH_INFO"))?,
    absolute_slot: (result
      |> u64_field("absoluteSlot", "EPOCH_INFO"))?
  })
end

pub fn latest_blockhash_from_response(response :: RpcResponse) -> LatestBlockhash!String do
  let result = (response
    |> require_rpc_result())?
  let context = Json.get(result, "context")
  let value = Json.get(result, "value")
  let blockhash = Json.get(value, "blockhash")
  if context == "" || value == "" || blockhash == "" do
    Err("SOLANA_LATEST_BLOCKHASH: missing context, value, or blockhash")
  else
    Ok(LatestBlockhash {
      context_slot: (context
        |> u64_field("slot", "LATEST_BLOCKHASH"))?,
      blockhash: (blockhash
        |> hash_value())?,
      last_valid_block_height: (value
        |> u64_field("lastValidBlockHeight", "LATEST_BLOCKHASH"))?
    })
  end
end

fn accounts_loop(values :: Json,
  index :: Int,
  total :: Int,
  accounts :: List<AccountInfo>) -> List<AccountInfo>!String do
  if index >= total do
    Ok(accounts)
  else
    let value = (values
      |> Json.array_get(index))?
    if value
      |> Json.is_null() do
      Err("SOLANA_RPC: account #{index} was not found")
    else
      accounts_loop(values,
        index + 1,
        total,
        List.append(accounts,
          (value
            |> Json.encode()
            |> account_info())?))
    end
  end
end

pub fn multiple_accounts_from_response(response :: RpcResponse) -> AccountsAtSlot!String do
  let result = Json.parse(require_rpc_result(response)?)?
  let context = Json.object_get(result, "context")?
  let values = Json.object_get(result, "value")?
  let total = (values
    |> Json.array_length())?
  Ok(AccountsAtSlot {
    slot: u64_text(Json.encode(Json.object_get(context, "slot")?), "RPC")?,
    accounts: (accounts_loop(values, 0, total, List.new()))?
  })
end

## Decode a sliced batch without erasing missing positions or allocation size.
pub fn optional_accounts_slice_from_response(response :: RpcResponse) -> OptionalAccountSlicesAtSlot!String do
  let result = Json.parse(require_rpc_result(response)?)?
  let context = Json.object_get(result, "context")?
  let slot = u64_text(Json.encode(Json.object_get(context, "slot")?), "RPC")?
  let values = Json.object_get(result, "value")?
  let count = Json.array_length(values)?
  if count > 100 do
    return Err("SOLANA_RPC: too many accounts")
  end
  let accounts = for index in 0..count do
    let value = Json.array_get(values, index)?
    if Json.is_null(value) do
      None
    else
      Some(account_slice(value)?)
    end
  end
  Ok(OptionalAccountSlicesAtSlot { slot: slot, accounts: accounts })
end

## Preserve missing entries and their positions; malformed entries and RPC
## errors remain errors. A missing account must not become an empty account.
pub fn optional_accounts_from_response(response :: RpcResponse) -> OptionalAccountsAtSlot!String do
  let result = Json.parse(require_rpc_result(response)?)?
  let context = Json.object_get(result, "context")?
  let slot = u64_text(Json.encode(Json.object_get(context, "slot")?), "RPC")?
  let values = Json.object_get(result, "value")?
  let count = Json.array_length(values)?
  if count > 100 do
    return Err("SOLANA_RPC: too many accounts")
  end
  let accounts = for index in 0..count do
    let value = Json.array_get(values, index)?
    if Json.is_null(value) do
      None
    else
      Some(account_info(Json.encode(value))?)
    end
  end
  Ok(OptionalAccountsAtSlot { slot: slot, accounts: accounts })
end

## Decode a withContext program-account response without assuming RPC ordering.
## Refuse duplicate addresses and responses exceeding the caller's bound rather
## than silently truncating a discovery result. HTTP byte limits remain required.
pub fn program_accounts_from_response(response :: RpcResponse,
  maximum_accounts :: Int) -> ProgramAccountsAtSlot!String do
  if maximum_accounts < 0 || maximum_accounts > 1000 do
    return Err("SOLANA_RPC: invalid program account bound")
  end
  let result = Json.parse(require_rpc_result(response)?)?
  let context = Json.object_get(result, "context")?
  let slot = u64_text(Json.encode(Json.object_get(context, "slot")?), "RPC")?
  let values = Json.object_get(result, "value")?
  let count = Json.array_length(values)?
  if count > maximum_accounts do
    return Err("SOLANA_RPC: too many program accounts")
  end
  let accounts = for index in 0..count do
    let item = Json.array_get(values, index)?
    let address = pubkey(Json.as_string(Json.object_get(item, "pubkey")?)?)?
    KeyedAccountInfo {
      address: address,
      account: account_info(Json.encode(Json.object_get(item, "account")?))?
    }
  end
  for index in 0..count do
    for previous in 0..index do
      if pubkey_equal(List.get(accounts, index).address, List.get(accounts, previous).address) do
        return Err("SOLANA_RPC: duplicate program account")
      end
    end
  end
  Ok(ProgramAccountsAtSlot { slot: slot, accounts: accounts })
end

pub fn rpc_send(client :: Int,
  url :: String,
  request :: RpcRequest,
  timeout_ms :: Int,
  max_response_bytes :: Int) -> RpcResponse!String do
  let response = (Http.build(:post, url)
    |> Http.header("Content-Type", "application/json")
    |> Http.body(request
      |> rpc_request_json())
    |> Http.timeout(timeout_ms)
    |> Http.max_redirects(0)
    |> Http.max_response_bytes(max_response_bytes) |2> Http.send_with(client))?
  if response.status != 200 do
    Err("SOLANA_RPC: HTTP status #{response.status}")
  else
    let parsed = rpc_response(response.body)?
    if parsed.id != request.id do
      Err("SOLANA_RPC: response id does not match request")
    else
      Ok(parsed)
    end
  end
end

pub fn account_notification(raw :: String) -> AccountNotification!String do
  (raw
    |> Json.parse())?
  if Json.get(raw, "method") != "accountNotification" do
    Err("SOLANA_WS: expected accountNotification")
  else
    let params = Json.get(raw, "params")
    let result = Json.get(params, "result")
    let context = Json.get(result, "context")
    Ok(AccountNotification {
      subscription: (params
        |> int_field("subscription", "WS"))?,
      slot: (context
        |> u64_field("slot", "WS"))?,
      account: (Json.get(result, "value")
        |> account_info())?
    })
  end
end

pub fn slot_notification(raw :: String) -> SlotNotification!String do
  (raw
    |> Json.parse())?
  if Json.get(raw, "method") != "slotNotification" do
    Err("SOLANA_WS: expected slotNotification")
  else
    let params = Json.get(raw, "params")
    let result = Json.get(params, "result")
    Ok(SlotNotification {
      subscription: (params
        |> int_field("subscription", "WS"))?,
      slot: (result
        |> u64_field("slot", "WS"))?,
      parent: (result
        |> u64_field("parent", "WS"))?,
      root: (result
        |> u64_field("root", "WS"))?
    })
  end
end
