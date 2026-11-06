from Solana.Read import (
  Pubkey,
  AccountInfo,
  SolanaClock,
  pubkey,
  pubkey_string,
  clock_sysvar_address,
  clock_sysvar
)

fn proof() -> Bool!String do
  assert(pubkey_string(clock_sysvar_address()?) == "SysvarC1ock11111111111111111111111111111111")
  let account = AccountInfo {
    owner: pubkey("Sysvar1111111111111111111111111111111111111")?,
    executable: false,
    lamports: U64.parse("1")?,
    rent_epoch: U64.parse("0")?,
    data: Bytes.from_hex("ffffffffffffffff0000000000000080ffffffffffffffffffffffffffffffffffffffffffffff7f")?
  }
  let clock = clock_sysvar(account)?
  assert(U64.to_string(clock.slot) == "18446744073709551615")
  assert(Int.to_string(clock.epoch_start_timestamp) == "-9223372036854775808")
  assert(U64.to_string(clock.epoch) == "18446744073709551615")
  assert(U64.to_string(clock.leader_schedule_epoch) == "18446744073709551615")
  assert(Int.to_string(clock.unix_timestamp) == "9223372036854775807")
  let negative = clock_sysvar(%{account | data: Bytes.from_hex(String.repeat("ff", 40))?})?
  assert(negative.epoch_start_timestamp == -1 && negative.unix_timestamp == -1)
  let zero = clock_sysvar(%{account | data: Bytes.from_hex(String.repeat("00", 40))?})?
  assert(zero.unix_timestamp == 0 && zero.epoch_start_timestamp == 0)
  for size in 0..40 do
    case clock_sysvar(%{account | data: Bytes.slice(account.data, 0, size)?}) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  for invalid in [
    %{account | data: Bytes.concat(account.data, Bytes.from_hex("00")?)?},
    %{account | executable: true},
    %{account | lamports: U64.parse("0")?},
    %{account | owner: pubkey("11111111111111111111111111111111")?}
  ] do
    case clock_sysvar(invalid) do
      Ok(_) -> assert(false)
      Err(_) -> nil
    end
  end
  Ok(true)
end

test("Clock sysvar decoding preserves signed timestamps and unsigned slots with exact ownership and layout") do
  case proof() do
    Err(error) -> do
      println(error)
      assert(false)
    end
    Ok(value) -> assert(value)
  end
end
