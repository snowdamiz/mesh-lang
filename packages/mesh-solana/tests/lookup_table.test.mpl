from Solana.Read import Pubkey, AccountInfo, pubkey
from Solana.Tx import Instruction
from Solana.LookupTable import (
  LookupTableAccount,
  LookupTableCreation,
  lookup_table_account,
  create_lookup_table_instruction,
  extend_lookup_table_instruction
)

fn proof() -> Bool!String do
  let key = pubkey("11111111111111111111111111111111")?
  let account = AccountInfo {
    data: Bytes.from_hex("01000000ffffffffffffffffffffffffffffffffff01"
      <> String.repeat("00", 34 + 32 * 256))?,
    owner: pubkey("AddressLookupTab1e1111111111111111111111111")?,
    executable: false,
    lamports: U64.parse("1")?,
    rent_epoch: U64.parse("0")?
  }
  let decoded = lookup_table_account(account)?
  assert(List.length(decoded.addresses) == 256)
  assert(decoded.last_extended_start_index == 255)
  assert(U64.to_string(decoded.last_extended_slot) == "18446744073709551615")
  assert(U64.to_string(decoded.deactivation_slot) == "18446744073709551615")
  for size in 0..56 do
    case lookup_table_account(%{account | data: Bytes.slice(account.data, 0, size)?}) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  for bad in [
    %{account | executable: true},
    %{account | lamports: U64.parse("0")?},
    %{account | owner: key},
    %{account | data: Bytes.concat(account.data, Bytes.from_hex("00")?)?},
    %{account | data: Bytes.concat(account.data, Bytes.from_hex(String.repeat("00", 32))?)?},
    %{account | data: Bytes.from_hex("00000000" <> String.repeat("00", 52))?},
    %{account |
      data: Bytes.from_hex("01000000"
        <> String.repeat("00", 17)
        <> "02"
        <> String.repeat("00", 34))?
    },
    %{account |
      data: Bytes.from_hex("01000000"
        <> String.repeat("00", 16)
        <> "0100"
        <> String.repeat("00", 34))?
    },
    %{account |
      data: Bytes.from_hex("01000000"
        <> String.repeat("00", 17)
        <> "01"
        <> String.repeat("00", 32)
        <> "0100")?
    },
    %{account |
      data: Bytes.from_hex("01000000"
        <> String.repeat("00", 18)
        <> "01"
        <> String.repeat("00", 33))?
    }
  ] do
    case lookup_table_account(bad) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  let invalid = Pubkey { bytes: Bytes.empty() }
  for authority in [invalid] do
    case create_lookup_table_instruction(authority, key, U64.parse("0")?) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  for addresses in [
    [],
    [invalid],
    for _ in 0..257 do
      key
    end
  ] do
    case extend_lookup_table_instruction(key, key, Some(key), addresses) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  Ok(true)
end

test("lookup table accounts and instructions reject malformed metadata, bounds and keys") do
  case proof() do
    Ok(value) -> assert(value)
    Err(error) -> do
      println(error)
      assert(false)
    end
  end
end
