#![cfg(unix)]

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "support/test_artifacts.rs"]
mod artifacts;

fn meshc_bin() -> PathBuf {
    artifacts::meshc_bin()
}

fn package_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("packages/mesh-solana")
}

fn account_json(data: &[u8], owner: &str, lamports: u64) -> String {
    json!({
        "data": [STANDARD.encode(data), "base64"],
        "executable": false,
        "lamports": lamports,
        "owner": owner,
        "rentEpoch": u64::MAX,
    })
    .to_string()
}

#[test]
fn solana_pda_and_curve_membership_match_the_pinned_official_sdk() {
    use sha2::{Digest, Sha256};
    use solana_address::Address;
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let mut curves = Vec::new();
    let mut addresses = Vec::new();
    for index in 0..256u16 {
        let candidates = [
            [index as u8; 32],
            Sha256::digest(index.to_le_bytes()).into(),
        ];
        for bytes in candidates {
            curves.push(json!({"bytes": hex(&bytes), "on_curve": Address::new_from_array(bytes).is_on_curve()}));
        }
        let program = Address::new_from_array(Sha256::digest(index.to_be_bytes()).into());
        let seeds = match index % 4 {
            0 => vec![],
            1 => vec![b"example".to_vec(), index.to_le_bytes().to_vec()],
            2 => vec![vec![index as u8; 32], vec![]],
            _ => vec![vec![index as u8; 32]; 15],
        };
        let refs: Vec<&[u8]> = seeds.iter().map(Vec::as_slice).collect();
        let (address, bump) = Address::find_program_address(&refs, &program);
        addresses.push(json!({"program": hex(program.as_ref()), "seeds": seeds.iter().map(|s| hex(s)).collect::<Vec<_>>(), "address": hex(address.as_ref()), "bump": bump}));
    }
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("solana-pda-proof");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("mesh.toml"), format!("[package]\nname = \"solana-pda-proof\"\nversion = \"0.1.0\"\n[dependencies]\nmesh-solana = {{ path = {:?} }}\n", package_source())).unwrap();
    let fixture_path = project.join("oracle.json");
    fs::write(
        &fixture_path,
        json!({"curves":curves,"addresses":addresses}).to_string(),
    )
    .unwrap();
    fs::write(
        project.join("main.mpl"),
        include_str!("../../../tests/e2e/solana_pda.mpl"),
    )
    .unwrap();
    let build = Command::new(meshc_bin())
        .args(["build", project.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(project.join("solana-pda-proof"))
        .env("MESH_SOLANA_PDA_FIXTURE", &fixture_path)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "solana-pda:ok\n");
}

#[test]
fn solana_read_package_decodes_rpc_spl_and_jitosol_state() {
    let package_text = fs::read_to_string(package_source().join("solana/read.mpl")).unwrap();
    let package_parse = mesh_parser::parse(&package_text);
    assert!(
        package_parse.errors().is_empty(),
        "package parse failed: {:#?}",
        package_parse.errors()
    );
    let package_typeck = mesh_typeck::check(&package_parse);
    assert!(
        package_typeck.errors.is_empty(),
        "package typecheck failed: {:#?}",
        package_typeck.errors
    );

    let mut pool = vec![0u8; 611];
    pool[0] = 1;
    pool[258..266].copy_from_slice(&12_345_678_900u64.to_le_bytes());
    pool[266..274].copy_from_slice(&10_000_000_000u64.to_le_bytes());
    pool[274..282].copy_from_slice(&777u64.to_le_bytes());

    let mut mint = vec![0u8; 82];
    mint[36..44].copy_from_slice(&10_000_000_000u64.to_le_bytes());
    mint[44] = 9;
    mint[45] = 1;

    let mut token = vec![0u8; 165];
    token[64..72].copy_from_slice(&2_000_000_000u64.to_le_bytes());
    token[72..76].copy_from_slice(&1u32.to_le_bytes());
    token[108] = 1;

    let pool_account = account_json(&pool, "SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy", 1);
    let mint_account = account_json(
        &mint,
        "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
        2_039_280,
    );
    let token_account = account_json(
        &token,
        "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
        2_039_280,
    );
    let rpc_response = json!({
        "jsonrpc": "2.0",
        "id": 9,
        "result": 320_000_006u64,
    })
    .to_string();
    let epoch_info_response = json!({
        "jsonrpc": "2.0",
        "id": 10,
        "result": {
            "absoluteSlot": 320_000_006u64,
            "epoch": 777u64,
        },
    })
    .to_string();
    let latest_blockhash_response = json!({
        "jsonrpc": "2.0",
        "id": 11,
        "result": {
            "context": { "slot": 320_000_006u64 },
            "value": {
                "blockhash": "11111111111111111111111111111111",
                "lastValidBlockHeight": 320_000_156u64,
            },
        },
    })
    .to_string();
    let multiple_accounts_response = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "context": { "slot": 320_000_006u64 },
            "value": [
                serde_json::from_str::<serde_json::Value>(&pool_account).unwrap(),
                serde_json::from_str::<serde_json::Value>(&mint_account).unwrap(),
            ],
        },
    })
    .to_string();
    let account_notification = json!({
        "jsonrpc": "2.0",
        "method": "accountNotification",
        "params": {
            "result": {
                "context": { "slot": 320_000_006u64 },
                "value": serde_json::from_str::<serde_json::Value>(&mint_account).unwrap(),
            },
            "subscription": 41,
        },
    })
    .to_string();
    let slot_notification = json!({
        "jsonrpc": "2.0",
        "method": "slotNotification",
        "params": {
            "result": {
                "parent": 320_000_005u64,
                "root": 319_999_974u64,
                "slot": 320_000_006u64,
            },
            "subscription": 42,
        },
    })
    .to_string();

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("solana-read-proof");
    let package = project.join(".mesh/packages/mesh-solana@0.2.0");
    fs::create_dir_all(package.join("solana")).unwrap();
    fs::copy(
        package_source().join("mesh.toml"),
        package.join("mesh.toml"),
    )
    .unwrap();
    fs::copy(
        package_source().join("solana/read.mpl"),
        package.join("solana/read.mpl"),
    )
    .unwrap();
    fs::write(
        project.join("mesh.toml"),
        "[package]\nname = \"solana-read-proof\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let source = r##"
from Solana.Read import AccountInfo, Mint, Pubkey, StakePoolState, account_info, account_notification, account_subscribe_request, block_height_from_response, data_size_filter, epoch_info_from_response, get_account_info_request, get_block_height_request, get_epoch_info_request, get_latest_blockhash_request, get_multiple_accounts_request, get_slot_request, hash_string, hash_value, jitosol_mint, jitosol_nav, jitosol_stake_pool, latest_blockhash_from_response, memcmp_filter, mint, multiple_accounts_from_response, program_accounts_request, pubkey, pubkey_string, rpc_request_json, rpc_response, signature, signature_string, slot_from_response, slot_notification, slot_subscribe_request, stake_pool, token_account

fn show_pubkey(value :: String) do
  case value |> pubkey() do
    Ok(key) -> key |> pubkey_string() |> println()
    Err(error) -> println(error)
  end
end

fn show_nav(
  pool_address :: Pubkey,
  pool_state :: StakePoolState,
  mint_address :: Pubkey,
  mint_state :: Mint,
  epoch :: U64
) do
  case jitosol_nav(pool_address, pool_state, mint_address, mint_state, epoch) do
    Ok(value) -> value |> U128.to_string() |> println()
    Err(error) -> println(error)
  end
end

fn proof() -> Int!String do
  let accounts = (("""__MULTIPLE_ACCOUNTS_RESPONSE__""" |> rpc_response())?
    |> multiple_accounts_from_response())?
  println("#{U64.to_string(accounts.slot)}:#{List.length(accounts.accounts)}")
  let pool_info = List.get(accounts.accounts, 0)
  let mint_info = List.get(accounts.accounts, 1)
  let token_info = ("""__TOKEN_ACCOUNT__""" |> account_info())?
  pool_info.rent_epoch |> U64.to_string() |> println()

  let pool_state = (pool_info |> stake_pool())?
  let mint_state = (mint_info |> mint())?
  let token_state = (token_info |> token_account())?
  pool_state.total_lamports |> U64.to_string() |> println()
  pool_state.pool_token_supply |> U64.to_string() |> println()
  pool_state.last_update_epoch |> U64.to_string() |> println()
  mint_state.supply |> U64.to_string() |> println()
  println("#{mint_state.decimals}:#{mint_state.initialized}")
  token_state.amount |> U64.to_string() |> println()
  println("#{token_state.state}")
  case token_state.delegate do
    Some(value) -> value |> pubkey_string() |> println()
    None -> println("no-delegate")
  end

  let pool_address = (jitosol_stake_pool())?
  let mint_address = (jitosol_mint())?
  let epoch = (U64.parse("777"))?
  show_nav(pool_address, pool_state, mint_address, mint_state, epoch)
  show_nav(pool_address, pool_state, mint_address, Mint {
    mint_authority: mint_state.mint_authority,
    supply: (U64.parse("9999999999"))?,
    decimals: mint_state.decimals,
    initialized: mint_state.initialized,
    freeze_authority: mint_state.freeze_authority
  }, epoch)
  show_nav(pool_address, pool_state, mint_address, Mint {
    mint_authority: mint_state.mint_authority,
    supply: (U64.parse("10000000001"))?,
    decimals: mint_state.decimals,
    initialized: mint_state.initialized,
    freeze_authority: mint_state.freeze_authority
  }, epoch)

  show_pubkey("0")
  let zero = "11111111111111111111111111111111"
  show_pubkey(zero)
  let sig = (signature("1111111111111111111111111111111111111111111111111111111111111111"))?
  sig |> signature_string() |> String.length() |> Int.to_string() |> println()
  let digest = (hash_value(zero))?
  digest.bytes |> Bytes.length() |> Int.to_string() |> println()

  let rpc = ("""__RPC_RESPONSE__""" |> rpc_response())?
  println("#{rpc.id}:#{rpc.ok}:#{rpc.result_json}")
  (rpc |> slot_from_response())?.value |> U64.to_string() |> println()
  (rpc |> block_height_from_response())?.value |> U64.to_string() |> println()

  let account_request = (7 |> get_account_info_request(mint_address, "confirmed"))?
  println("#{account_request.method}:#{account_request.params_json}")
  account_request |> rpc_request_json() |> println()
  let multiple = (8 |> get_multiple_accounts_request([pool_address, mint_address], "confirmed"))?
  println("#{multiple.method}:#{multiple.params_json}")
  let slot_request = (9 |> get_slot_request("finalized"))?
  println("#{slot_request.method}:#{slot_request.params_json}")
  let height_request = (10 |> get_block_height_request("processed"))?
  println("#{height_request.method}:#{height_request.params_json}")
  let epoch_request = (11 |> get_epoch_info_request("confirmed"))?
  println("#{epoch_request.method}:#{epoch_request.params_json}")
  let epoch_info = ((("""__EPOCH_INFO_RESPONSE__""" |> rpc_response())?)
    |> epoch_info_from_response())?
  println("#{U64.to_string(epoch_info.epoch)}:#{U64.to_string(epoch_info.absolute_slot)}")
  let blockhash_request = (11 |> get_latest_blockhash_request("confirmed"))?
  println("#{blockhash_request.method}:#{blockhash_request.params_json}")
  let latest_blockhash = ((("""__LATEST_BLOCKHASH_RESPONSE__""" |> rpc_response())?)
    |> latest_blockhash_from_response())?
  println("#{U64.to_string(latest_blockhash.context_slot)}:#{hash_string(latest_blockhash.blockhash)}:#{U64.to_string(latest_blockhash.last_valid_block_height)}")
  let filters = [
    (memcmp_filter(0, "11111111111111111111111111111111"))?,
    (data_size_filter(611))?
  ]
  let programs = (12 |> program_accounts_request(pool_address, filters, "confirmed"))?
  println("#{programs.method}:#{programs.params_json}")
  let account_sub = (12 |> account_subscribe_request(mint_address, "confirmed"))?
  println("#{account_sub.method}:#{account_sub.params_json}")
  let slot_sub = slot_subscribe_request(13)
  println("#{slot_sub.method}:#{slot_sub.params_json}")

  let account_event = ("""__ACCOUNT_NOTIFICATION__""" |> account_notification())?
  println("#{account_event.subscription}:#{U64.to_string(account_event.slot)}:#{Bytes.length(account_event.account.data)}")
  let slot_event = ("""__SLOT_NOTIFICATION__""" |> slot_notification())?
  println("#{slot_event.subscription}:#{U64.to_string(slot_event.slot)}:#{U64.to_string(slot_event.parent)}:#{U64.to_string(slot_event.root)}")
  Ok(0)
end

fn main() do
  case proof() do
    Ok(_) -> println("done")
    Err(error) -> println(error)
  end
end
"##
    .replace("__TOKEN_ACCOUNT__", &token_account)
    .replace("__RPC_RESPONSE__", &rpc_response)
    .replace("__EPOCH_INFO_RESPONSE__", &epoch_info_response)
    .replace("__LATEST_BLOCKHASH_RESPONSE__", &latest_blockhash_response)
    .replace(
        "__MULTIPLE_ACCOUNTS_RESPONSE__",
        &multiple_accounts_response,
    )
    .replace("__ACCOUNT_NOTIFICATION__", &account_notification)
    .replace("__SLOT_NOTIFICATION__", &slot_notification);
    fs::write(project.join("main.mpl"), source).unwrap();

    let build = Command::new(meshc_bin())
        .args(["build", project.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(project.join("solana-read-proof"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "Solana read proof failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "320000006:2\n18446744073709551615\n12345678900\n10000000000\n777\n10000000000\n9:true\n2000000000\n1\n11111111111111111111111111111111\n1234567890\n1234567890\nSOLANA_JITOSOL: mint supply exceeds stake pool accounting\nSOLANA_PUBKEY: invalid base58\n11111111111111111111111111111111\n64\n32\n9:true:320000006\n320000006\n320000006\ngetAccountInfo:[\"J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn\",{\"commitment\":\"confirmed\",\"encoding\":\"base64\"}]\n{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"getAccountInfo\",\"params\":[\"J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn\",{\"commitment\":\"confirmed\",\"encoding\":\"base64\"}]}\ngetMultipleAccounts:[[\"Jito4APyf642JPZPx3hGc6WWJ8zPKtRbRs4P815Awbb\",\"J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn\"],{\"commitment\":\"confirmed\",\"encoding\":\"base64\"}]\ngetSlot:[{\"commitment\":\"finalized\"}]\ngetBlockHeight:[{\"commitment\":\"processed\"}]\ngetEpochInfo:[{\"commitment\":\"confirmed\"}]\n777:320000006\ngetLatestBlockhash:[{\"commitment\":\"confirmed\"}]\n320000006:11111111111111111111111111111111:320000156\ngetProgramAccounts:[\"Jito4APyf642JPZPx3hGc6WWJ8zPKtRbRs4P815Awbb\",{\"commitment\":\"confirmed\",\"encoding\":\"base64\",\"filters\":[{\"memcmp\":{\"offset\":0,\"bytes\":\"11111111111111111111111111111111\"}},{\"dataSize\":611}]}]\naccountSubscribe:[\"J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn\",{\"commitment\":\"confirmed\",\"encoding\":\"base64\"}]\nslotSubscribe:[]\n41:320000006:82\n42:320000006:320000005:319999974\ndone\n"
    );
}

#[test]
fn solana_tx_package_inspects_jupiter_instruction() {
    let tx_text = fs::read_to_string(package_source().join("solana/tx.mpl")).unwrap();
    let tx_parse = mesh_parser::parse(&tx_text);
    assert!(
        tx_parse.errors().is_empty(),
        "transaction package parse failed: {:#?}",
        tx_parse.errors()
    );

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("solana-tx-proof");
    let package = project.join(".mesh/packages/mesh-solana@0.2.0");
    fs::create_dir_all(package.join("solana")).unwrap();
    fs::copy(
        package_source().join("mesh.toml"),
        package.join("mesh.toml"),
    )
    .unwrap();
    fs::copy(
        package_source().join("solana/read.mpl"),
        package.join("solana/read.mpl"),
    )
    .unwrap();
    fs::copy(
        package_source().join("solana/tx.mpl"),
        package.join("solana/tx.mpl"),
    )
    .unwrap();
    fs::write(
        project.join("mesh.toml"),
        "[package]\nname = \"solana-tx-proof\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(
        project.join("main.mpl"),
        r#"
from Solana.Read import Hash, Pubkey
from Solana.Tx import AccountMeta, AddressTableLookup, CompiledInstruction, Instruction, LegacyMessage, MessageHeader, MessageV0, compile_legacy_message, compile_message_v0, instruction_from_jupiter_json, instruction_report_json, jupiter_instruction_set_from_json, jupiter_instruction_set_report_json, jupiter_instructions, legacy_message_report_json, message_v0_report_json, serialize_legacy_message, serialize_message_v0, serialize_unsigned_legacy_transaction, simulate_transaction_request

fn proof() -> Int!String do
  case """{"programId":"ComputeBudget111111111111111111111111111111","accounts":[],"data":"AQID"}"""
    |> instruction_from_jupiter_json() do
    Err(error) -> println(error)
    Ok(instruction) -> instruction
      |> instruction_report_json()
      |> println()
  end

  let jupiter = ("""{"computeBudgetInstructions":[{"programId":"ComputeBudget111111111111111111111111111111","accounts":[],"data":"AQID"}],"setupInstructions":[],"swapInstruction":{"programId":"JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4","accounts":[{"pubkey":"11111111111111111111111111111111","isSigner":true,"isWritable":true}],"data":"BAUG"},"cleanupInstruction":null,"otherInstructions":[],"addressesByLookupTableAddress":{}}"""
    |> jupiter_instruction_set_from_json())?
  if List.length(jupiter_instructions(jupiter)) != 2 do
    return Err("Jupiter instruction order differs")
  end
  jupiter
    |> jupiter_instruction_set_report_json()
    |> println()

  let message = LegacyMessage {
    header: MessageHeader {
      num_required_signatures: 1,
      num_readonly_signed_accounts: 0,
      num_readonly_unsigned_accounts: 1
    },
    account_keys: [
      Pubkey { bytes: ("0000000000000000000000000000000000000000000000000000000000000000" |> Bytes.from_hex())? },
      Pubkey { bytes: ("0101010101010101010101010101010101010101010101010101010101010101" |> Bytes.from_hex())? }
    ],
    recent_blockhash: Hash { bytes: ("0202020202020202020202020202020202020202020202020202020202020202" |> Bytes.from_hex())? },
    instructions: [
      CompiledInstruction {
        program_id_index: 1,
        account_indexes: [0],
        data: ("0201010000" |> Bytes.from_hex())?
      }
    ]
  }
  let compiled_message = (message.account_keys
    |> List.get(0)
    |> compile_legacy_message(
      message.recent_blockhash,
      [
        Instruction {
          program_id: message.account_keys
            |> List.get(1),
          accounts: [
            AccountMeta {
              pubkey: message.account_keys
                |> List.get(0),
              signer: true,
              writable: true
            }
          ],
          data: ("0201010000"
            |> Bytes.from_hex())?
        }
      ]
    ))?
  let compiled_legacy_hex = (compiled_message
    |> serialize_legacy_message()) ?
    |> Bytes.to_hex()
  let manual_legacy_hex = (message
    |> serialize_legacy_message()) ?
    |> Bytes.to_hex()
  if compiled_legacy_hex != manual_legacy_hex do
    return Err("compiled legacy message differs")
  end
  case message
    |> serialize_legacy_message() do
    Ok(bytes) -> bytes
      |> Bytes.to_hex()
      |> println()
    Err(error) -> println(error)
  end
  (message |> legacy_message_report_json())? |> println()
  let unsigned = (message |> serialize_unsigned_legacy_transaction())?
  let simulation = (unsigned |2> simulate_transaction_request(
    7,
    "confirmed",
    false,
    None
  ))?
  println(simulation.method <> ":" <> Int.to_string(Bytes.length(unsigned)))

  let v0 = MessageV0 {
    header: message.header,
    static_account_keys: message.account_keys,
    recent_blockhash: message.recent_blockhash,
    instructions: [
      CompiledInstruction {
        program_id_index: 1,
        account_indexes: [0, 2],
        data: ("aabb" |> Bytes.from_hex())?
      }
    ],
    address_table_lookups: [
      AddressTableLookup {
        account_key: Pubkey { bytes: ("0303030303030303030303030303030303030303030303030303030303030303" |> Bytes.from_hex())? },
        writable_indexes: [4],
        readonly_indexes: [5],
        writable_addresses: [
          Pubkey { bytes: ("0404040404040404040404040404040404040404040404040404040404040404" |> Bytes.from_hex())? }
        ],
        readonly_addresses: [
          Pubkey { bytes: ("0505050505050505050505050505050505050505050505050505050505050505" |> Bytes.from_hex())? }
        ]
      }
    ]
  }
  let v0_lookup = v0.address_table_lookups
    |> List.get(0)
  let compiled_v0 = (message.account_keys
    |> List.get(0)
    |> compile_message_v0(
      message.recent_blockhash,
      [
        Instruction {
          program_id: message.account_keys
            |> List.get(1),
          accounts: [
            AccountMeta {
              pubkey: message.account_keys
                |> List.get(0),
              signer: true,
              writable: true
            },
            AccountMeta {
              pubkey: v0_lookup.writable_addresses
                |> List.get(0),
              signer: false,
              writable: true
            }
          ],
          data: ("aabb"
            |> Bytes.from_hex())?
        }
      ],
      v0.address_table_lookups
    ))?
  let compiled_v0_hex = (compiled_v0
    |> serialize_message_v0()) ?
    |> Bytes.to_hex()
  let manual_v0_hex = (v0
    |> serialize_message_v0()) ?
    |> Bytes.to_hex()
  if compiled_v0_hex != manual_v0_hex do
    return Err("compiled v0 message differs")
  end
  let duplicate_rejected = case message.account_keys
    |> List.get(0)
    |> compile_message_v0(
      message.recent_blockhash,
      [
        Instruction {
          program_id: message.account_keys
            |> List.get(1),
          accounts: [
            AccountMeta {
              pubkey: v0_lookup.writable_addresses
                |> List.get(0),
              signer: false,
              writable: true
            }
          ],
          data: ("aabb"
            |> Bytes.from_hex())?
        }
      ],
      [
        AddressTableLookup {
          account_key: v0_lookup.account_key,
          writable_indexes: v0_lookup.writable_indexes,
          readonly_indexes: v0_lookup.readonly_indexes,
          writable_addresses: v0_lookup.writable_addresses,
          readonly_addresses: v0_lookup.writable_addresses
        }
      ]
    ) do
    Err(_) -> true
    Ok(_) -> false
  end
  if !duplicate_rejected do
    return Err("duplicate loaded address accepted")
  end
  case v0
    |> serialize_message_v0() do
    Ok(bytes) -> bytes
      |> Bytes.to_hex()
      |> println()
    Err(error) -> println(error)
  end
  (v0 |> message_v0_report_json())? |> println()
  Ok(0)
end

fn main() do
  case proof() do
    Ok(_) -> 0
    Err(error) -> do
      println(error)
      1
    end
  end
end
"#,
    )
    .unwrap();

    let build = Command::new(meshc_bin())
        .args(["build", project.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "meshc build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(project.join("solana-tx-proof"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "Solana transaction proof failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        concat!(
            "{\"schemaVersion\":1,\"programId\":\"ComputeBudget111111111111111111111111111111\",\"accountCount\":0,\"accountKeys\":[],\"signerKeys\":[],\"writableKeys\":[],\"dataBase64\":\"AQID\",\"dataBytes\":3}\n",
            "{\"schemaVersion\":1,\"source\":\"jupiter-build\",\"instructionCount\":2,\"computeBudgetCount\":1,\"setupCount\":0,\"otherCount\":0,\"cleanupCount\":0,\"tipCount\":0,\"dataBytes\":6,\"programIds\":[\"ComputeBudget111111111111111111111111111111\",\"JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4\"],\"accountKeys\":[\"11111111111111111111111111111111\"],\"signerKeys\":[\"11111111111111111111111111111111\"],\"writableKeys\":[\"11111111111111111111111111111111\"]}\n",
            "0100010200000000000000000000000000000000000000000000000000000000000000000101010101010101010101010101010101010101010101010101010101010101020202020202020202020202020202020202020202020202020202020202020201010100050201010000\n",
            "{\"schemaVersion\":1,\"version\":\"legacy\",\"requiredSignatures\":1,\"accountKeys\":[\"11111111111111111111111111111111\",\"4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi\"],\"programIds\":[\"4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi\"],\"instructionCount\":1,\"lookupTableKeys\":[],\"loadedWritableAccounts\":0,\"loadedReadonlyAccounts\":0,\"messageBytes\":110}\n",
            "simulateTransaction:175\n",
            "8001000102000000000000000000000000000000000000000000000000000000000000000001010101010101010101010101010101010101010101010101010101010101010202020202020202020202020202020202020202020202020202020202020202010102000202aabb01030303030303030303030303030303030303030303030303030303030303030301040105\n",
            "{\"schemaVersion\":1,\"version\":\"v0\",\"requiredSignatures\":1,\"staticAccountKeys\":[\"11111111111111111111111111111111\",\"4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi\"],\"accountKeys\":[\"11111111111111111111111111111111\",\"4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi\",\"GgBaCs3NCBuZN12kCJgAW63ydqohFkHEdfdEXBPzLHq\",\"LbUiWL3xVV8hTFYBVdbTNrpDo41NKS6o3LHHuDzjfcY\"],\"programIds\":[\"4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi\"],\"instructionCount\":1,\"lookupTableKeys\":[\"CktRuQ2mttgRGkXJtyksdKHjUdc2C4TgDzyB98oEzy8\"],\"loadedWritableAccounts\":1,\"loadedReadonlyAccounts\":1,\"loadedWritableAccountKeys\":[\"GgBaCs3NCBuZN12kCJgAW63ydqohFkHEdfdEXBPzLHq\"],\"loadedReadonlyAccountKeys\":[\"LbUiWL3xVV8hTFYBVdbTNrpDo41NKS6o3LHHuDzjfcY\"],\"messageBytes\":146}\n",
        )
    );
}

#[test]
fn solana_lookup_table_instructions_and_accounts_match_the_pinned_sdk() {
    use solana_address::Address;
    use solana_address_lookup_table_interface::{
        instruction::*,
        state::{AddressLookupTable, LookupTableMeta},
    };
    use std::borrow::Cow;
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let mut cases = Vec::new();
    for index in 0..64u8 {
        let authority = Address::new_from_array([index; 32]);
        let payer = Address::new_from_array([index.wrapping_add(1); 32]);
        let slot = match index % 4 {
            0 => 0,
            1 => u64::MAX,
            2 => 9007199254740993,
            _ => u64::from(index),
        };
        let (create, table) = create_lookup_table(authority, payer, slot);
        let addresses: Vec<_> = (0..=index)
            .map(|n| Address::new_from_array([n; 32]))
            .collect();
        let mut item = json!({"authority":authority.to_string(),"payer":payer.to_string(),"slot":slot.to_string(),"table":table.to_string(),"addresses":addresses.iter().map(ToString::to_string).collect::<Vec<_>>()});
        for (name, ix) in [
            ("create", create),
            (
                "extend",
                extend_lookup_table(table, authority, Some(payer), addresses.clone()),
            ),
            (
                "extend_no_payer",
                extend_lookup_table(table, authority, None, addresses.clone()),
            ),
            ("deactivate", deactivate_lookup_table(table, authority)),
            ("close", close_lookup_table(table, authority, payer)),
        ] {
            item[name] = json!({"program":ix.program_id.to_string(),"data":STANDARD.encode(ix.data),
                "accounts":ix.accounts.iter().map(|a|json!({"address":a.pubkey.to_string(),"signer":a.is_signer,"writable":a.is_writable})).collect::<Vec<_>>()});
        }
        for (name, authority) in [("account", Some(authority)), ("frozen_account", None)] {
            item[name] = hex(&AddressLookupTable {
                meta: LookupTableMeta {
                    authority,
                    last_extended_slot: slot,
                    ..LookupTableMeta::default()
                },
                addresses: Cow::Borrowed(&addresses),
            }
            .serialize_for_tests()
            .unwrap())
            .into();
        }
        cases.push(item);
    }
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("solana-lookup-proof");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("mesh.toml"),format!("[package]\nname = \"solana-lookup-proof\"\nversion = \"0.1.0\"\n[dependencies]\nmesh-solana = {{ path = {:?} }}\n",package_source())).unwrap();
    let fixture = project.join("oracle.json");
    fs::write(&fixture, serde_json::to_vec(&cases).unwrap()).unwrap();
    fs::write(
        project.join("main.mpl"),
        include_str!("../../../tests/e2e/solana_lookup_table.mpl"),
    )
    .unwrap();
    let build = Command::new(meshc_bin())
        .args(["build", project.to_str().unwrap(), "--opt-level", "2"])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(project.join("solana-lookup-proof"))
        .env("MESH_SOLANA_LOOKUP_FIXTURE", fixture)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stdout), "solana-lookup:ok\n");
}

#[test]
fn solana_sdk_account_serialization_preserves_fixed_wire_vectors_and_padding() {
    use solana_address_lookup_table_interface::state::{AddressLookupTable, LookupTableMeta};
    use solana_slot_hashes::SlotHashes;
    use std::borrow::Cow;
    let decode = |hex: &str| {
        hex.as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect::<Vec<_>>()
    };
    // Fixed legacy Solana account bytes: u64 count, then (u64 slot, 32-byte hash).
    // The high slot distinguishes fixed-width u64 from a lossy floating-point or
    // variable-length codec. Nonzero account padding must remain acceptable.
    let hashes = SlotHashes::new(&[
        (1, [0x11; 32].into()),
        (9_007_199_254_740_993, [0x22; 32].into()),
    ]);
    let expected = decode(concat!(
        "0200000000000000",
        "0100000000002000",
        "2222222222222222222222222222222222222222222222222222222222222222",
        "0100000000000000",
        "1111111111111111111111111111111111111111111111111111111111111111",
    ));
    assert_eq!(wincode::serialize(&hashes).unwrap(), expected);
    let mut padded = expected.clone();
    padded.resize(solana_slot_hashes::SIZE, 0xa5);
    assert_eq!(wincode::deserialize::<SlotHashes>(&padded).unwrap(), hashes);
    assert!(wincode::deserialize_exact::<SlotHashes>(&padded).is_err());
    assert!(wincode::deserialize::<SlotHashes>(&expected[..expected.len() - 1]).is_err());

    let addresses = [[0x44; 32].into()];
    let table = AddressLookupTable {
        meta: LookupTableMeta {
            deactivation_slot: u64::MAX,
            last_extended_slot: 9_007_199_254_740_993,
            last_extended_slot_start_index: 3,
            authority: Some([0x33; 32].into()),
            _padding: 0,
        },
        addresses: Cow::Borrowed(&addresses),
    };
    assert_eq!(
        table.serialize_for_tests().unwrap(),
        decode(concat!(
            "01000000",
            "ffffffffffffffff",
            "0100000000002000",
            "0301",
            "3333333333333333333333333333333333333333333333333333333333333333",
            "0000",
            "4444444444444444444444444444444444444444444444444444444444444444",
        ))
    );
}

#[test]
fn solana_slot_hashes_and_lookup_cooldown_match_the_pinned_sdk() {
    use solana_address_lookup_table_interface::state::{
        AddressLookupTable, LookupTableMeta, LookupTableStatus,
    };
    use solana_slot_hashes::SlotHashes;
    use std::borrow::Cow;
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let mut cases = Vec::new();
    for current in [0u64, 1, 10_000, 9_007_199_254_740_993, u64::MAX] {
        for count in [0usize, 1, 2, 512] {
            let newest = current.saturating_sub(1);
            if count > 0 && newest < (count as u64 - 1) * 2 {
                continue;
            }
            let pairs: Vec<_> = (0..count)
                .map(|n| (newest - n as u64 * 2, [n as u8; 32].into()))
                .collect();
            let hashes = SlotHashes::new(&pairs);
            let compact = wincode::serialize(&hashes).unwrap();
            let mut padded = compact.clone();
            padded.resize(solana_slot_hashes::SIZE, 0xa5);
            assert_eq!(wincode::deserialize::<SlotHashes>(&padded).unwrap(), hashes);
            let oldest = pairs.last().map_or(0, |value| value.0);
            let mut deactivations = vec![
                u64::MAX,
                current,
                newest,
                current.saturating_sub(2),
                oldest,
                oldest.saturating_sub(1),
                0,
            ];
            deactivations.sort_unstable();
            deactivations.dedup();
            for deactivation_slot in deactivations {
                let meta = LookupTableMeta {
                    deactivation_slot,
                    ..LookupTableMeta::default()
                };
                let (status, remaining) = match meta.status(current, &hashes) {
                    LookupTableStatus::Activated => ("active", 0),
                    LookupTableStatus::Deactivating { remaining_blocks } => {
                        ("cooling", remaining_blocks)
                    }
                    LookupTableStatus::Deactivated => ("closed", 0),
                };
                let table = AddressLookupTable {
                    meta,
                    addresses: Cow::Borrowed(&[]),
                }
                .serialize_for_tests()
                .unwrap();
                cases.push(json!({"current":current.to_string(),"compact":hex(&compact),"padded":hex(&padded),
                    "entries":pairs.iter().map(|(slot,hash)|json!({"slot":slot.to_string(),"hash":hex(hash.as_ref())})).collect::<Vec<_>>(),
                    "table":hex(&table),"status":status,"remaining":remaining}));
            }
        }
    }
    assert!(cases.len() > 64);
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("solana-slot-hashes-proof");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("mesh.toml"),format!("[package]\nname = \"solana-slot-hashes-proof\"\nversion = \"0.1.0\"\n[dependencies]\nmesh-solana = {{ path = {:?} }}\n",package_source())).unwrap();
    let fixture = project.join("oracle.json");
    fs::write(&fixture, serde_json::to_vec(&cases).unwrap()).unwrap();
    fs::write(
        project.join("main.mpl"),
        include_str!("../../../tests/e2e/solana_slot_hashes.mpl"),
    )
    .unwrap();
    let build = Command::new(meshc_bin())
        .args(["build", project.to_str().unwrap(), "--opt-level", "2"])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(project.join("solana-slot-hashes-proof"))
        .env("MESH_SLOT_HASHES_FIXTURE", fixture)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "solana-slot-hashes:ok\n"
    );
}
