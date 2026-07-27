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
  accounts :: List < AccountInfo >
end

pub struct TokenAccount do
  mint :: Pubkey
  owner :: Pubkey
  amount :: U64
  delegate :: Option < Pubkey >
  state :: Int
  native_reserve :: Option < U64 >
  delegated_amount :: U64
  close_authority :: Option < Pubkey >
end

pub struct Mint do
  mint_authority :: Option < Pubkey >
  supply :: U64
  decimals :: Int
  initialized :: Bool
  freeze_authority :: Option < Pubkey >
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

fn fixed_base58(value :: String, length :: Int, label :: String) -> Bytes ! String do
  case value
    |> Bytes.from_base58() do
    Err( _) -> Err("SOLANA_#{label}: invalid base58")
    Ok( bytes) -> if Bytes.length(bytes) != length do
      Err("SOLANA_#{label}: expected #{length} bytes, got #{Bytes.length(bytes)}")
    else
      if (bytes
        |> Bytes.to_base58()) == value do
        Ok(bytes)
      else
        Err("SOLANA_#{label}: non-canonical base58")
      end
    end
  end
end

fn pubkey_from_bytes(bytes :: Bytes) -> Pubkey ! String do
  if Bytes.length(bytes) == 32 do
    Ok(Pubkey { bytes : bytes })
  else
    Err("SOLANA_PUBKEY: expected 32 bytes, got #{Bytes.length(bytes)}")
  end
end

pub fn pubkey(value :: String) -> Pubkey ! String do
  Ok(Pubkey { bytes : (value
    |> fixed_base58(32, "PUBKEY")) ? })
end

pub fn signature(value :: String) -> Signature ! String do
  Ok(Signature { bytes : (value
    |> fixed_base58(64, "SIGNATURE")) ? })
end

pub fn hash_value(value :: String) -> Hash ! String do
  Ok(Hash { bytes : (value
    |> fixed_base58(32, "HASH")) ? })
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

pub fn slot(value :: String) -> Slot ! String do
  Ok(Slot { value : (value
    |> U64.parse()) ? })
end

pub fn block_height(value :: String) -> BlockHeight ! String do
  Ok(BlockHeight { value : (value
    |> U64.parse()) ? })
end

pub fn spl_token_program() -> Pubkey ! String do
  "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
    |> pubkey()
end

pub fn spl_stake_pool_program() -> Pubkey ! String do
  "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy"
    |> pubkey()
end

pub fn jitosol_stake_pool() -> Pubkey ! String do
  "Jito4APyf642JPZPx3hGc6WWJ8zPKtRbRs4P815Awbb"
    |> pubkey()
end

pub fn jitosol_mint() -> Pubkey ! String do
  "J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn"
    |> pubkey()
end

fn u64_text(value :: String, label :: String) -> U64 ! String do
  case value
    |> U64.parse() do
    Ok( parsed) -> Ok(parsed)
    Err( _) -> Err("SOLANA_#{label}: expected an unsigned integer")
  end
end

fn u64_field(raw :: String, field :: String, label :: String) -> U64 ! String do
  let value = Json.get(raw, field)
  if value == "" do
    Err("SOLANA_#{label}: missing field #{field}")
  else
    value
      |> u64_text(label)
  end
end

fn int_field(raw :: String, field :: String, label :: String) -> Int ! String do
  case raw
    |> Json.get(field)
    |> String.to_int() do
    Some( parsed) -> if parsed >= 0 do
      Ok(parsed)
    else
      Err("SOLANA_#{label}: field #{field} must be non-negative")
    end
    None -> Err("SOLANA_#{label}: field #{field} must be an integer")
  end
end

fn account_data(encoded :: String) -> Bytes ! String do
  case encoded
    |> Bytes.from_base64() do
    Err( _) -> Err("SOLANA_ACCOUNT: invalid base64 data")
    Ok( decoded) -> if (decoded
      |> Bytes.to_base64()) == encoded do
      Ok(decoded)
    else
      Err("SOLANA_ACCOUNT: non-canonical base64 data")
    end
  end
end

pub fn account_info(raw :: String) -> AccountInfo ! String do
  let root = (raw
    |> Json.parse()) ?
  let data = (root
    |> Json.object_get("data")) ?
  if (data
    |> Json.array_length()) ? != 2 do
    Err("SOLANA_ACCOUNT: data must be [payload, encoding]")
  else
    let encoded = ((data
      |> Json.array_get(0)) ?
      |> Json.as_string()) ?
    let encoding = ((data
      |> Json.array_get(1)) ?
      |> Json.as_string()) ?
    if encoding != "base64" do
      Err("SOLANA_ACCOUNT: data encoding must be base64")
    else
      let owner_text = Json.get(raw, "owner")
      if owner_text == "" do
        Err("SOLANA_ACCOUNT: missing owner")
      else
        Ok(AccountInfo {
          data : (encoded
            |> account_data()) ?,
          executable : ((root
            |> Json.object_get("executable")) ?
            |> Json.as_bool()) ?,
          lamports : (raw
            |> u64_field("lamports", "ACCOUNT")) ?,
          owner : (owner_text
            |> pubkey()) ?,
          rent_epoch : (raw
            |> u64_field("rentEpoch", "ACCOUNT")) ?
        })
      end
    end
  end
end

fn read_u64(data :: Bytes, offset :: Int, label :: String) -> U64 ! String do
  case data
    |> Bytes.read_uint_le(offset, 8) do
    Ok( value) -> value
      |> u64_text(label)
    Err( _) -> Err("SOLANA_#{label}: u64 read is out of bounds")
  end
end

fn read_byte(data :: Bytes, offset :: Int, label :: String) -> Int ! String do
  case data
    |> Bytes.get(offset) do
    Ok( value) -> Ok(value)
    Err( _) -> Err("SOLANA_#{label}: byte read is out of bounds")
  end
end

fn coption_pubkey(data :: Bytes, offset :: Int, label :: String) -> Option < Pubkey > ! String do
  case data
    |> Bytes.read_uint_le(offset, 4) do
    Ok( "0") -> Ok(None)
    Ok( "1") -> case data
      |> Bytes.slice(offset + 4, 32) do
      Err( _) -> Err("SOLANA_#{label}: COption read is out of bounds")
      Ok( bytes) -> case bytes
        |> pubkey_from_bytes() do
        Err( error) -> Err(error)
        Ok( key) -> Ok(Some(key))
      end
    end
    Ok( tag) -> Err("SOLANA_#{label}: invalid COption tag #{tag}")
    Err( _) -> Err("SOLANA_#{label}: COption read is out of bounds")
  end
end

fn coption_u64(data :: Bytes, offset :: Int, label :: String) -> Option < U64 > ! String do
  case data
    |> Bytes.read_uint_le(offset, 4) do
    Ok( "0") -> Ok(None)
    Ok( "1") -> Ok(Some((data
      |> read_u64(offset + 4, label)) ?))
    Ok( tag) -> Err("SOLANA_#{label}: invalid COption tag #{tag}")
    Err( _) -> Err("SOLANA_#{label}: COption read is out of bounds")
  end
end

fn validate_owner(account :: AccountInfo, expected :: Pubkey, label :: String) -> Int ! String do
  if account.owner
    |> pubkey_equal(expected) do
    Ok(0)
  else
    Err("SOLANA_#{label}: account owner mismatch")
  end
end

pub fn token_account(account :: AccountInfo) -> TokenAccount ! String do
  (account
    |> validate_owner((spl_token_program()) ?, "TOKEN_ACCOUNT")) ?
  if Bytes.length(account.data) != 165 do
    Err("SOLANA_TOKEN_ACCOUNT: expected 165 bytes, got #{Bytes.length(account.data)}")
  else
    let state = (account.data
      |> read_byte(108, "TOKEN_ACCOUNT")) ?
    if state < 0 || state > 2 do
      Err("SOLANA_TOKEN_ACCOUNT: invalid state #{state}")
    else
      Ok(TokenAccount {
        mint : ((account.data
          |> Bytes.slice(0, 32)) ?
          |> pubkey_from_bytes()) ?,
        owner : ((account.data
          |> Bytes.slice(32, 32)) ?
          |> pubkey_from_bytes()) ?,
        amount : (account.data
          |> read_u64(64, "TOKEN_ACCOUNT")) ?,
        delegate : (account.data
          |> coption_pubkey(72, "TOKEN_ACCOUNT")) ?,
        state : state,
        native_reserve : (account.data
          |> coption_u64(109, "TOKEN_ACCOUNT")) ?,
        delegated_amount : (account.data
          |> read_u64(121, "TOKEN_ACCOUNT")) ?,
        close_authority : (account.data
          |> coption_pubkey(129, "TOKEN_ACCOUNT")) ?
      })
    end
  end
end

pub fn mint(account :: AccountInfo) -> Mint ! String do
  (account
    |> validate_owner((spl_token_program()) ?, "MINT")) ?
  if Bytes.length(account.data) != 82 do
    Err("SOLANA_MINT: expected 82 bytes, got #{Bytes.length(account.data)}")
  else
    let initialized = (account.data
      |> read_byte(45, "MINT")) ?
    if initialized != 0 && initialized != 1 do
      Err("SOLANA_MINT: initialized flag must be 0 or 1")
    else
      Ok(Mint {
        mint_authority : (account.data
          |> coption_pubkey(0, "MINT")) ?,
        supply : (account.data
          |> read_u64(36, "MINT")) ?,
        decimals : (account.data
          |> read_byte(44, "MINT")) ?,
        initialized : initialized == 1,
        freeze_authority : (account.data
          |> coption_pubkey(46, "MINT")) ?
      })
    end
  end
end

pub fn stake_pool(account :: AccountInfo) -> StakePoolState ! String do
  (account
    |> validate_owner((spl_stake_pool_program()) ?, "STAKE_POOL")) ?
  if Bytes.length(account.data) != 611 do
    Err("SOLANA_STAKE_POOL: expected 611 bytes, got #{Bytes.length(account.data)}")
  else
    let account_type = (account.data
      |> read_byte(0, "STAKE_POOL")) ?
    if account_type != 1 do
      Err("SOLANA_STAKE_POOL: unsupported account type #{account_type}")
    else
      Ok(StakePoolState {
        account_type : account_type,
        total_lamports : (account.data
          |> read_u64(258, "STAKE_POOL")) ?,
        pool_token_supply : (account.data
          |> read_u64(266, "STAKE_POOL")) ?,
        last_update_epoch : (account.data
          |> read_u64(274, "STAKE_POOL")) ?
      })
    end
  end
end

fn validate_jitosol(pool_address :: Pubkey,
pool :: StakePoolState,
mint_address :: Pubkey,
mint_state :: Mint,
current_epoch :: U64) -> Int ! String do
  if !pubkey_equal(pool_address, (jitosol_stake_pool()) ?) do
    Err("SOLANA_JITOSOL: unexpected stake-pool address")
  else
    if !pubkey_equal(mint_address, (jitosol_mint()) ?) do
      Err("SOLANA_JITOSOL: unexpected mint address")
    else
      if U64.compare(pool.total_lamports, (U64.parse("0")) ?) <= 0 do
        Err("SOLANA_JITOSOL: total pool lamports must be positive")
      else
        if U64.compare(pool.pool_token_supply, (U64.parse("0")) ?) <= 0 do
          Err("SOLANA_JITOSOL: pool token supply must be positive")
        else
          if U64.compare(mint_state.supply, pool.pool_token_supply) != 0 do
            Err("SOLANA_JITOSOL: mint supply does not match stake pool")
          else
            if U64.compare(pool.last_update_epoch, current_epoch) != 0 do
              Err("SOLANA_JITOSOL: stake pool is stale for the current epoch")
            else
              if mint_state.decimals != 9 || !mint_state.initialized do
                Err("SOLANA_JITOSOL: unexpected mint configuration")
              else
                Ok(0)
              end
            end
          end
        end
      end
    end
  end
end

pub fn jitosol_nav(pool_address :: Pubkey,
pool :: StakePoolState,
mint_address :: Pubkey,
mint_state :: Mint,
current_epoch :: U64) -> U128 ! String do
  (validate_jitosol(pool_address, pool, mint_address, mint_state, current_epoch)) ?
  let total = (pool.total_lamports
    |> U64.to_string()
    |> U128.parse()) ?
  let supply = (pool.pool_token_supply
    |> U64.to_string()
    |> U128.parse()) ?
  ((total
    |> U128.multiply((U128.parse("1000000000")) ?)) ?
    |> U128.divide(supply))
end

fn validate_commitment(value :: String) -> Int ! String do
  if value == "processed" || value == "confirmed" || value == "finalized" do
    Ok(0)
  else
    Err("SOLANA_RPC: unsupported commitment #{value}")
  end
end

pub fn rpc_request(id :: Int, method :: String, params_json :: String) -> RpcRequest ! String do
  if id < 0 do
    Err("SOLANA_RPC: request id must be non-negative")
  else
    if String.length(method) == 0 do
      Err("SOLANA_RPC: method must not be empty")
    else
      case params_json
        |> Json.parse() do
        Ok( _) -> Ok(RpcRequest {
          id : id,
          method : method,
          params_json : params_json
        })
        Err( _) -> Err("SOLANA_RPC: params must be valid JSON")
      end
    end
  end
end

pub fn rpc_request_json(request :: RpcRequest) -> String do
  """{"jsonrpc":"2.0","id":#{request.id},"method":#{Json.encode_string(request.method)},"params":#{request.params_json}}"""
end

fn account_params(address :: Pubkey, commitment :: String) -> String ! String do
  (commitment
    |> validate_commitment()) ?
  Ok("[#{Json.encode_string(pubkey_string(address))},{\"commitment\":#{Json.encode_string(commitment)},\"encoding\":\"base64\"}]")
end

pub fn get_account_info_request(id :: Int, address :: Pubkey, commitment :: String) -> RpcRequest ! String do
  rpc_request(id,
  "getAccountInfo",
  (address
    |> account_params(commitment)) ?)
end

fn pubkeys_json_loop(values :: List < Pubkey >, index :: Int, total :: Int, output :: String) -> String do
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

fn pubkeys_json(values :: List < Pubkey >) -> String do
  pubkeys_json_loop(values, 0, List.length(values), "[")
end

pub fn get_multiple_accounts_request(id :: Int, addresses :: List < Pubkey >, commitment :: String) -> RpcRequest ! String do
  (commitment
    |> validate_commitment()) ?
  if List.length(addresses) == 0 do
    Err("SOLANA_RPC: getMultipleAccounts requires at least one address")
  else
    rpc_request(id,
    "getMultipleAccounts",
    "[#{pubkeys_json(addresses)},{\"commitment\":#{Json.encode_string(commitment)},\"encoding\":\"base64\"}]")
  end
end

fn commitment_params(commitment :: String) -> String ! String do
  (commitment
    |> validate_commitment()) ?
  Ok("[{\"commitment\":#{Json.encode_string(commitment)}}]")
end

pub fn get_slot_request(id :: Int, commitment :: String) -> RpcRequest ! String do
  rpc_request(id,
  "getSlot",
  (commitment
    |> commitment_params()) ?)
end

pub fn get_block_height_request(id :: Int, commitment :: String) -> RpcRequest ! String do
  rpc_request(id,
  "getBlockHeight",
  (commitment
    |> commitment_params()) ?)
end

pub fn get_epoch_info_request(id :: Int, commitment :: String) -> RpcRequest ! String do
  rpc_request(id,
  "getEpochInfo",
  (commitment
    |> commitment_params()) ?)
end

pub fn get_latest_blockhash_request(id :: Int, commitment :: String) -> RpcRequest ! String do
  rpc_request(id,
  "getLatestBlockhash",
  (commitment
    |> commitment_params()) ?)
end

pub fn memcmp_filter(offset :: Int, bytes :: String) -> ProgramAccountFilter ! String do
  if offset < 0 do
    Err("SOLANA_FILTER: memcmp offset must be non-negative")
  else
    case bytes
      |> Bytes.from_base58() do
      Err( _) -> Err("SOLANA_FILTER: memcmp bytes must be base58")
      Ok( decoded) -> if Bytes.length(decoded) > 128 do
        Err("SOLANA_FILTER: memcmp bytes exceed 128 decoded bytes")
      else
        Ok(ProgramAccountFilter { encoded : "{\"memcmp\":{\"offset\":#{offset},\"bytes\":#{Json.encode_string(bytes)}}}" })
      end
    end
  end
end

pub fn data_size_filter(size :: Int) -> ProgramAccountFilter ! String do
  if size < 0 do
    Err("SOLANA_FILTER: data size must be non-negative")
  else
    Ok(ProgramAccountFilter { encoded : "{\"dataSize\":#{size}}" })
  end
end

fn filters_json_loop(values :: List < ProgramAccountFilter >,
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

pub fn filters_json(values :: List < ProgramAccountFilter >) -> String do
  filters_json_loop(values, 0, List.length(values), "[")
end

pub fn program_accounts_request(id :: Int,
program :: Pubkey,
filters :: List < ProgramAccountFilter >,
commitment :: String) -> RpcRequest ! String do
  (commitment
    |> validate_commitment()) ?
  rpc_request(id,
  "getProgramAccounts",
  "[#{Json.encode_string(pubkey_string(program))},{\"commitment\":#{Json.encode_string(commitment)},\"encoding\":\"base64\",\"filters\":#{filters_json(filters)}}]")
end

pub fn account_subscribe_request(id :: Int, address :: Pubkey, commitment :: String) -> RpcRequest ! String do
  rpc_request(id,
  "accountSubscribe",
  (address
    |> account_params(commitment)) ?)
end

pub fn slot_subscribe_request(id :: Int) -> RpcRequest do
  RpcRequest {
    id : id,
    method : "slotSubscribe",
    params_json : "[]"
  }
end

pub fn program_subscribe_request(id :: Int,
program :: Pubkey,
filters :: List < ProgramAccountFilter >,
commitment :: String) -> RpcRequest ! String do
  (commitment
    |> validate_commitment()) ?
  rpc_request(id,
  "programSubscribe",
  "[#{Json.encode_string(pubkey_string(program))},{\"commitment\":#{Json.encode_string(commitment)},\"encoding\":\"base64\",\"filters\":#{filters_json(filters)}}]")
end

pub fn send_account_subscription(connection :: Int,
id :: Int,
address :: Pubkey,
commitment :: String) do
  ((id
    |> account_subscribe_request(address, commitment)) ?
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
filters :: List < ProgramAccountFilter >,
commitment :: String) do
  ((id
    |> program_subscribe_request(program, filters, commitment)) ?
    |> rpc_request_json() |2> WsClient.send_text(connection))
end

pub fn rpc_response(raw :: String) -> RpcResponse ! String do
  (raw
    |> Json.parse()) ?
  if Json.get(raw, "jsonrpc") != "2.0" do
    Err("SOLANA_RPC: response jsonrpc must be 2.0")
  else
    let id = (raw
      |> int_field("id", "RPC")) ?
    let error = Json.get(raw, "error")
    let result = Json.get(raw, "result")
    if error != "" && error != "null" do
      Ok(RpcResponse {
        id : id,
        ok : false,
        result_json : "",
        error_json : error
      })
    else
      if result == "" do
        Err("SOLANA_RPC: response has neither result nor error")
      else
        Ok(RpcResponse {
          id : id,
          ok : true,
          result_json : result,
          error_json : ""
        })
      end
    end
  end
end

fn require_rpc_result(response :: RpcResponse) -> String ! String do
  if response.ok do
    Ok(response.result_json)
  else
    Err("SOLANA_RPC: #{response.error_json}")
  end
end

pub fn slot_from_response(response :: RpcResponse) -> Slot ! String do
  Ok(Slot { value : ((response
    |> require_rpc_result()) ?
    |> u64_text("SLOT")) ? })
end

pub fn block_height_from_response(response :: RpcResponse) -> BlockHeight ! String do
  Ok(BlockHeight { value : ((response
    |> require_rpc_result()) ?
    |> u64_text("BLOCK_HEIGHT")) ? })
end

pub fn epoch_info_from_response(response :: RpcResponse) -> EpochInfo ! String do
  let result = (response
    |> require_rpc_result()) ?
  Ok(EpochInfo {
    epoch : (result
      |> u64_field("epoch", "EPOCH_INFO")) ?,
    absolute_slot : (result
      |> u64_field("absoluteSlot", "EPOCH_INFO")) ?
  })
end

pub fn latest_blockhash_from_response(response :: RpcResponse) -> LatestBlockhash ! String do
  let result = (response
    |> require_rpc_result()) ?
  let context = Json.get(result, "context")
  let value = Json.get(result, "value")
  let blockhash = Json.get(value, "blockhash")
  if context == "" || value == "" || blockhash == "" do
    Err("SOLANA_LATEST_BLOCKHASH: missing context, value, or blockhash")
  else
    Ok(LatestBlockhash {
      context_slot : (context
        |> u64_field("slot", "LATEST_BLOCKHASH")) ?,
      blockhash : (blockhash
        |> hash_value()) ?,
      last_valid_block_height : (value
        |> u64_field("lastValidBlockHeight", "LATEST_BLOCKHASH")) ?
    })
  end
end

fn accounts_loop(values :: Json, index :: Int, total :: Int, accounts :: List < AccountInfo >) -> List < AccountInfo > ! String do
  if index >= total do
    Ok(accounts)
  else
    let value = (values
      |> Json.array_get(index)) ?
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
        |> account_info()) ?))
    end
  end
end

pub fn multiple_accounts_from_response(response :: RpcResponse) -> AccountsAtSlot ! String do
  let result = (response
    |> require_rpc_result()) ?
  let context = Json.get(result, "context")
  let values = (Json.get(result, "value")
    |> Json.parse()) ?
  let total = (values
    |> Json.array_length()) ?
  Ok(AccountsAtSlot {
    slot : (context
      |> u64_field("slot", "RPC")) ?,
    accounts : (accounts_loop(values, 0, total, List.new())) ?
  })
end

pub fn rpc_send(client :: Int,
url :: String,
request :: RpcRequest,
timeout_ms :: Int,
max_response_bytes :: Int) -> RpcResponse ! String do
  let response = (Http.build(:post, url)
    |> Http.header("Content-Type", "application/json")
    |> Http.body(request
      |> rpc_request_json())
    |> Http.timeout(timeout_ms)
    |> Http.max_response_bytes(max_response_bytes) |2> Http.send_with(client)) ?
  if response.status != 200 do
    Err("SOLANA_RPC: HTTP status #{response.status}")
  else
    response.body
      |> rpc_response()
  end
end

pub fn account_notification(raw :: String) -> AccountNotification ! String do
  (raw
    |> Json.parse()) ?
  if Json.get(raw, "method") != "accountNotification" do
    Err("SOLANA_WS: expected accountNotification")
  else
    let params = Json.get(raw, "params")
    let result = Json.get(params, "result")
    let context = Json.get(result, "context")
    Ok(AccountNotification {
      subscription : (params
        |> int_field("subscription", "WS")) ?,
      slot : (context
        |> u64_field("slot", "WS")) ?,
      account : (Json.get(result, "value")
        |> account_info()) ?
    })
  end
end

pub fn slot_notification(raw :: String) -> SlotNotification ! String do
  (raw
    |> Json.parse()) ?
  if Json.get(raw, "method") != "slotNotification" do
    Err("SOLANA_WS: expected slotNotification")
  else
    let params = Json.get(raw, "params")
    let result = Json.get(params, "result")
    Ok(SlotNotification {
      subscription : (params
        |> int_field("subscription", "WS")) ?,
      slot : (result
        |> u64_field("slot", "WS")) ?,
      parent : (result
        |> u64_field("parent", "WS")) ?,
      root : (result
        |> u64_field("root", "WS")) ?
    })
  end
end
